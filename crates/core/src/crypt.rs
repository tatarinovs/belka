//! Encryption in the rclone crypt format, so that files encrypted here can be
//! read with `rclone` and the other way round:
//! https://rclone.org/crypt/#file-formats
//!
//! * Keys: scrypt(password, salt, N=16384, r=8, p=1) gives 80 bytes: the
//!   content key, the name key and the name tweak.
//! * Content: an 8-byte magic and a random 24-byte nonce, then the data in
//!   64 KiB blocks, each sealed with XSalsa20-Poly1305 (NaCl secretbox). The
//!   nonce is incremented for every block.
//! * Names: every path segment is PKCS#7-padded, encrypted with AES-256 in
//!   EME mode and written in lowercase base32hex without padding.

use crate::client::{Sink, Source, SourceReader};
use aes::cipher::{BlockDecrypt, BlockEncrypt};
use aes::Aes256;
use anyhow::{bail, ensure, Context, Result};
use async_trait::async_trait;
use crypto_secretbox::aead::AeadInPlace;
use crypto_secretbox::{KeyInit, Tag, XSalsa20Poly1305};
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context as TaskContext, Poll};
use tokio::io::{AsyncRead, ReadBuf};

const MAGIC: &[u8; 8] = b"RCLONE\x00\x00";
const NONCE_SIZE: usize = 24;
const HEADER_SIZE: u64 = (MAGIC.len() + NONCE_SIZE) as u64;
const TAG_SIZE: usize = 16;
const BLOCK_DATA: usize = 64 * 1024;
const BLOCK_SIZE: usize = TAG_SIZE + BLOCK_DATA;

/// The salt rclone uses when no second password is set.
const DEFAULT_SALT: [u8; 16] = [
    0xA8, 0x0D, 0xF4, 0x3A, 0x8F, 0xBD, 0x03, 0x08, 0xA7, 0xCA, 0xB8, 0x3E, 0x58, 0x1F, 0x86, 0xB1,
];

/// What a user sets up for an encrypted folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CryptConfig {
    pub password: String,
    /// rclone's `password2`. Empty means rclone's built-in salt.
    #[serde(default)]
    pub salt: String,
    /// rclone's `filename_encryption = standard`; otherwise `off`, where
    /// names stay readable and files get the `.bin` suffix.
    pub encrypt_names: bool,
}

pub struct Cipher {
    content: XSalsa20Poly1305,
    names: Aes256,
    tweak: [u8; 16],
    encrypt_names: bool,
}

impl Cipher {
    pub fn new(config: &CryptConfig) -> Result<Self> {
        ensure!(
            !config.password.is_empty(),
            "The encryption password is empty"
        );
        let salt = if config.salt.is_empty() {
            &DEFAULT_SALT[..]
        } else {
            config.salt.as_bytes()
        };
        let mut key = [0u8; 80];
        // The length here only matters for password hashes; the output is `key`.
        let params = scrypt::Params::new(14, 8, 1, 32).expect("valid scrypt parameters");
        scrypt::scrypt(config.password.as_bytes(), salt, &params, &mut key)
            .expect("valid scrypt output length");
        Ok(Self {
            content: XSalsa20Poly1305::new(key[..32].into()),
            names: Aes256::new(key[32..64].into()),
            tweak: key[64..80].try_into().expect("16 bytes"),
            encrypt_names: config.encrypt_names,
        })
    }

    // ---- names ----

    pub fn encrypt_file_name(&self, name: &str) -> String {
        if self.encrypt_names {
            self.encrypt_segment(name)
        } else {
            format!("{name}.bin")
        }
    }

    /// `None` for names that were not encrypted with this key.
    pub fn decrypt_file_name(&self, name: &str) -> Option<String> {
        if self.encrypt_names {
            self.decrypt_segment(name).ok()
        } else {
            name.strip_suffix(".bin")
                .filter(|n| !n.is_empty())
                .map(str::to_string)
        }
    }

    pub fn encrypt_dir_name(&self, name: &str) -> String {
        if self.encrypt_names {
            self.encrypt_segment(name)
        } else {
            name.to_string()
        }
    }

    pub fn decrypt_dir_name(&self, name: &str) -> Option<String> {
        if self.encrypt_names {
            self.decrypt_segment(name).ok()
        } else {
            Some(name.to_string())
        }
    }

    fn encrypt_segment(&self, plain: &str) -> String {
        if plain.is_empty() {
            return String::new();
        }
        let mut data = plain.as_bytes().to_vec();
        let pad = 16 - data.len() % 16;
        data.resize(data.len() + pad, pad as u8);
        eme(&self.names, &self.tweak, &mut data, true);
        base32hex_encode(&data)
    }

    fn decrypt_segment(&self, encrypted: &str) -> Result<String> {
        if encrypted.is_empty() {
            return Ok(String::new());
        }
        let mut data = base32hex_decode(encrypted).context("Not an encrypted name")?;
        ensure!(
            !data.is_empty() && data.len() % 16 == 0 && data.len() <= 16 * 128,
            "Not an encrypted name"
        );
        eme(&self.names, &self.tweak, &mut data, false);
        let pad = *data.last().expect("not empty") as usize;
        ensure!(
            (1..=16).contains(&pad) && data[data.len() - pad..].iter().all(|&b| b as usize == pad),
            "Not an encrypted name"
        );
        data.truncate(data.len() - pad);
        String::from_utf8(data).context("Not an encrypted name")
    }

    // ---- content ----

    /// Encrypts data held in memory into a whole encrypted file.
    pub fn encrypt_bytes(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0u8; NONCE_SIZE];
        getrandom::fill(&mut nonce).map_err(|err| anyhow::anyhow!("No random source: {err}"))?;
        let mut out = Vec::with_capacity(encrypted_size(data.len() as u64) as usize);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        for chunk in data.chunks(BLOCK_DATA) {
            let mut block = chunk.to_vec();
            let tag = self.seal(&nonce, &mut block);
            out.extend_from_slice(&tag);
            out.extend_from_slice(&block);
            increment(&mut nonce);
        }
        Ok(out)
    }

    /// Decrypts a whole encrypted file held in memory.
    pub fn decrypt_bytes(&self, file: &[u8]) -> Result<Vec<u8>> {
        let header = HEADER_SIZE as usize;
        let Some(size) = decrypted_size(file.len() as u64).filter(|_| file.starts_with(MAGIC))
        else {
            bail!("The file is not encrypted or is damaged");
        };
        let mut nonce: [u8; NONCE_SIZE] = file[MAGIC.len()..header].try_into().expect("24 bytes");
        let mut out = Vec::with_capacity(size as usize);
        for block in file[header..].chunks(BLOCK_SIZE) {
            let (tag, data) = block.split_at(TAG_SIZE);
            let mut data = data.to_vec();
            self.open(&nonce, tag, &mut data)?;
            out.extend_from_slice(&data);
            increment(&mut nonce);
        }
        Ok(out)
    }

    fn seal(&self, nonce: &[u8; NONCE_SIZE], data: &mut [u8]) -> Tag {
        self.content
            .encrypt_in_place_detached(nonce.into(), b"", data)
            .expect("a block is far below the cipher's length limit")
    }

    fn open(&self, nonce: &[u8; NONCE_SIZE], tag: &[u8], data: &mut [u8]) -> Result<()> {
        self.content
            .decrypt_in_place_detached(nonce.into(), b"", data, Tag::from_slice(tag))
            .map_err(|_| anyhow::anyhow!("Wrong password, or the encrypted file is damaged"))
    }
}

/// Data of another [`Source`], encrypted on the fly. All blocks use nonces
/// derived from one random nonce, so the data reads the same from any offset
/// and a failed upload can be resumed.
pub struct EncryptedSource<'a> {
    plain: &'a dyn Source,
    cipher: Arc<Cipher>,
    nonce: [u8; NONCE_SIZE],
}

impl<'a> EncryptedSource<'a> {
    pub fn new(plain: &'a dyn Source, cipher: Arc<Cipher>) -> Result<Self> {
        let mut nonce = [0u8; NONCE_SIZE];
        getrandom::fill(&mut nonce).map_err(|err| anyhow::anyhow!("No random source: {err}"))?;
        Ok(Self {
            plain,
            cipher,
            nonce,
        })
    }
}

impl Source for EncryptedSource<'_> {
    fn len(&self) -> u64 {
        encrypted_size(self.plain.len())
    }

    fn open_at(&self, offset: u64) -> Result<SourceReader> {
        let (block, skip, out) = match offset.checked_sub(HEADER_SIZE) {
            None => {
                let mut header = MAGIC.to_vec();
                header.extend_from_slice(&self.nonce);
                (0, offset as usize, header)
            }
            Some(rest) => (
                rest / BLOCK_SIZE as u64,
                (rest % BLOCK_SIZE as u64) as usize,
                Vec::new(),
            ),
        };
        let mut nonce = self.nonce;
        add(&mut nonce, block);
        Ok(Box::new(EncryptReader {
            plain: self.plain.open_at(block * BLOCK_DATA as u64)?,
            cipher: self.cipher.clone(),
            nonce,
            block: vec![0; BLOCK_DATA],
            filled: 0,
            eof: false,
            out,
            out_pos: 0,
            skip,
        }))
    }
}

struct EncryptReader {
    plain: SourceReader,
    cipher: Arc<Cipher>,
    /// Nonce of the next block.
    nonce: [u8; NONCE_SIZE],
    /// Plain data of the next block.
    block: Vec<u8>,
    filled: usize,
    eof: bool,
    /// Encrypted bytes not yet handed out.
    out: Vec<u8>,
    out_pos: usize,
    /// Encrypted bytes still to drop to reach the requested offset.
    skip: usize,
}

impl AsyncRead for EncryptReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        loop {
            let skipped = this.skip.min(this.out.len() - this.out_pos);
            this.out_pos += skipped;
            this.skip -= skipped;
            if this.out_pos < this.out.len() {
                let n = buf.remaining().min(this.out.len() - this.out_pos);
                buf.put_slice(&this.out[this.out_pos..this.out_pos + n]);
                this.out_pos += n;
                return Poll::Ready(Ok(()));
            }
            if this.eof {
                return Poll::Ready(Ok(()));
            }
            while this.filled < BLOCK_DATA {
                let mut read = ReadBuf::new(&mut this.block[this.filled..]);
                ready!(Pin::new(&mut this.plain).poll_read(cx, &mut read))?;
                match read.filled().len() {
                    0 => {
                        this.eof = true;
                        break;
                    }
                    n => this.filled += n,
                }
            }
            if this.filled == 0 {
                continue;
            }
            let data = &mut this.block[..this.filled];
            let tag = this.cipher.seal(&this.nonce, data);
            this.out.clear();
            this.out.extend_from_slice(&tag);
            this.out.extend_from_slice(data);
            this.out_pos = 0;
            this.filled = 0;
            increment(&mut this.nonce);
        }
    }
}

/// Decrypts a download on the fly into another [`Sink`]. The undecrypted
/// tail stays in memory, so a resumed download continues where it stopped.
pub struct DecryptSink<'a> {
    out: &'a mut dyn Sink,
    cipher: Arc<Cipher>,
    /// Nonce of the next block, once the header has arrived.
    nonce: Option<[u8; NONCE_SIZE]>,
    pending: Vec<u8>,
    received: u64,
    written: u64,
}

impl<'a> DecryptSink<'a> {
    pub fn new(cipher: Arc<Cipher>, out: &'a mut dyn Sink) -> Self {
        Self {
            out,
            cipher,
            nonce: None,
            pending: Vec::new(),
            received: 0,
            written: 0,
        }
    }

    /// Decrypts the last block once the download is complete. Returns the
    /// size of the decrypted data.
    pub async fn finish(mut self) -> Result<u64> {
        if self.nonce.is_none() {
            bail!("The file is not encrypted or is damaged");
        }
        if !self.pending.is_empty() {
            let len = self.pending.len();
            ensure!(len > TAG_SIZE, "The encrypted file is damaged");
            self.decrypt_block(len).await?;
        }
        Ok(self.written)
    }

    async fn decrypt_block(&mut self, len: usize) -> Result<()> {
        let nonce = self.nonce.as_mut().expect("header read");
        let (tag, data) = self.pending[..len].split_at_mut(TAG_SIZE);
        self.cipher.open(nonce, tag, data)?;
        increment(nonce);
        self.out.write(&self.pending[TAG_SIZE..len]).await?;
        self.written += (len - TAG_SIZE) as u64;
        self.pending.drain(..len);
        Ok(())
    }
}

#[async_trait]
impl Sink for DecryptSink<'_> {
    fn position(&self) -> u64 {
        self.received
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        self.received += data.len() as u64;
        self.pending.extend_from_slice(data);
        if self.nonce.is_none() {
            if self.pending.len() < HEADER_SIZE as usize {
                return Ok(());
            }
            if &self.pending[..MAGIC.len()] != MAGIC {
                bail!("The file is not encrypted or is damaged");
            }
            let nonce = self.pending[MAGIC.len()..HEADER_SIZE as usize]
                .try_into()
                .expect("24 bytes");
            self.nonce = Some(nonce);
            self.pending.drain(..HEADER_SIZE as usize);
        }
        while self.pending.len() >= BLOCK_SIZE {
            self.decrypt_block(BLOCK_SIZE).await?;
        }
        Ok(())
    }

    async fn reset(&mut self) -> Result<()> {
        self.nonce = None;
        self.pending.clear();
        self.received = 0;
        self.written = 0;
        self.out.reset().await
    }
}

const SEALED_MAGIC: &[u8; 8] = b"BELKA\x00\x00\x01";
const SEALED_SALT: usize = 16;

/// Encrypts `data` with a password alone, for files such as an export of
/// accounts: a magic, a random salt, then an encrypted file in the rclone
/// format with the salt in hex as `password2`. The random salt keeps equal
/// passwords from giving equal keys. Key derivation is deliberately slow.
pub fn seal(password: &str, data: &[u8]) -> Result<Vec<u8>> {
    let mut salt = [0u8; SEALED_SALT];
    getrandom::fill(&mut salt).map_err(|err| anyhow::anyhow!("No random source: {err}"))?;
    let mut out = SEALED_MAGIC.to_vec();
    out.extend_from_slice(&salt);
    out.extend(sealed_cipher(password, &salt)?.encrypt_bytes(data)?);
    Ok(out)
}

/// Opens a file made by [`seal`].
pub fn unseal(password: &str, file: &[u8]) -> Result<Vec<u8>> {
    let Some(rest) = file.strip_prefix(SEALED_MAGIC) else {
        bail!("Not a file of this program, or it is damaged");
    };
    ensure!(rest.len() >= SEALED_SALT, "The file is damaged");
    let (salt, data) = rest.split_at(SEALED_SALT);
    sealed_cipher(password, salt)?.decrypt_bytes(data)
}

fn sealed_cipher(password: &str, salt: &[u8]) -> Result<Cipher> {
    Cipher::new(&CryptConfig {
        password: password.to_string(),
        salt: hex::encode(salt),
        encrypt_names: false,
    })
}

/// Size of a file after encryption.
pub fn encrypted_size(size: u64) -> u64 {
    let (blocks, rest) = (size / BLOCK_DATA as u64, size % BLOCK_DATA as u64);
    HEADER_SIZE + blocks * BLOCK_SIZE as u64 + if rest > 0 { TAG_SIZE as u64 + rest } else { 0 }
}

/// Size of the data in an encrypted file; `None` if no encrypted file has this size.
pub fn decrypted_size(size: u64) -> Option<u64> {
    let size = size.checked_sub(HEADER_SIZE)?;
    let (blocks, rest) = (size / BLOCK_SIZE as u64, size % BLOCK_SIZE as u64);
    let tail = match rest {
        0 => 0,
        r if r > TAG_SIZE as u64 => r - TAG_SIZE as u64,
        _ => return None,
    };
    Some(blocks * BLOCK_DATA as u64 + tail)
}

/// Adds one to a little-endian number.
fn increment(nonce: &mut [u8]) {
    add(nonce, 1);
}

/// Adds `n` to a little-endian number.
fn add(nonce: &mut [u8], mut n: u64) {
    let mut carry = 0u16;
    for byte in nonce {
        if n == 0 && carry == 0 {
            break;
        }
        let sum = u16::from(*byte) + (n & 0xff) as u16 + carry;
        *byte = sum as u8;
        carry = sum >> 8;
        n >>= 8;
    }
}

// ---- EME (ECB-Mix-ECB) wide-block mode, as in github.com/rfjakob/eme ----

/// Multiplication by 2 in GF(2^128), little-endian as in EME.
fn times_two(block: &[u8; 16]) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[0] = block[0] << 1;
    if block[15] >= 0x80 {
        out[0] ^= 0x87;
    }
    for j in 1..16 {
        out[j] = (block[j] << 1) | (block[j - 1] >> 7);
    }
    out
}

fn xor(a: &[u8; 16], b: &[u8]) -> [u8; 16] {
    std::array::from_fn(|i| a[i] ^ b[i])
}

/// Encrypts or decrypts `data` (1 to 128 blocks of 16 bytes) in place.
fn eme(aes: &Aes256, tweak: &[u8; 16], data: &mut [u8], encrypt: bool) {
    let transform = |block: &mut [u8; 16]| {
        let b = block.into();
        if encrypt {
            aes.encrypt_block(b)
        } else {
            aes.decrypt_block(b)
        }
    };
    let m = data.len() / 16;

    let mut l = [0u8; 16];
    aes.encrypt_block((&mut l).into());
    let mut table = Vec::with_capacity(m);
    for _ in 0..m {
        l = times_two(&l);
        table.push(l);
    }

    let mut blocks: Vec<[u8; 16]> = data
        .chunks_exact(16)
        .zip(&table)
        .map(|(p, l)| {
            let mut pp = xor(l, p);
            transform(&mut pp);
            pp
        })
        .collect();

    let mut mp = xor(&blocks[0], tweak);
    for block in &blocks[1..] {
        mp = xor(&mp, block);
    }
    let mut mc = mp;
    transform(&mut mc);
    let mut mask = xor(&mp, &mc);
    for block in &mut blocks[1..] {
        mask = times_two(&mask);
        *block = xor(block, &mask);
    }
    let mut first = xor(&mc, tweak);
    for block in &blocks[1..] {
        first = xor(&first, block);
    }
    blocks[0] = first;

    for ((out, block), l) in data.chunks_exact_mut(16).zip(&mut blocks).zip(&table) {
        transform(block);
        out.copy_from_slice(&xor(block, l));
    }
}

// ---- base32hex (RFC 4648), lowercase, no padding ----

const BASE32HEX: &[u8; 32] = b"0123456789abcdefghijklmnopqrstuv";

fn base32hex_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
    let (mut buffer, mut bits) = (0u64, 0u32);
    for &byte in data {
        buffer = (buffer << 8) | byte as u64;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(BASE32HEX[(buffer >> bits) as usize & 31] as char);
        }
    }
    if bits > 0 {
        out.push(BASE32HEX[(buffer << (5 - bits)) as usize & 31] as char);
    }
    out
}

fn base32hex_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 5 / 8);
    let (mut buffer, mut bits) = (0u64, 0u32);
    for c in text.bytes() {
        let value = match c.to_ascii_lowercase() {
            c @ b'0'..=b'9' => c - b'0',
            c @ b'a'..=b'v' => c - b'a' + 10,
            _ => return None,
        };
        buffer = (buffer << 5) | value as u64;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    // Leftover bits must be zero padding, or the text is not canonical.
    (bits < 5 && buffer & ((1 << bits) - 1) == 0).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{BytesSource, MemorySink};
    use tokio::io::AsyncReadExt;

    fn cipher_with(password: &str, salt: &str, encrypt_names: bool) -> Arc<Cipher> {
        let config = CryptConfig {
            password: password.into(),
            salt: salt.into(),
            encrypt_names,
        };
        Arc::new(Cipher::new(&config).unwrap())
    }

    fn cipher(encrypt_names: bool) -> Arc<Cipher> {
        cipher_with("password", "", encrypt_names)
    }

    async fn read_all(source: &dyn Source, offset: u64) -> Vec<u8> {
        let mut out = Vec::new();
        source
            .open_at(offset)
            .unwrap()
            .read_to_end(&mut out)
            .await
            .unwrap();
        out
    }

    /// Decrypts `data` fed to the sink in pieces of `piece` bytes.
    async fn decrypt(c: &Arc<Cipher>, data: &[u8], piece: usize) -> Result<Vec<u8>> {
        let mut out = MemorySink::default();
        let mut sink = DecryptSink::new(c.clone(), &mut out);
        for chunk in data.chunks(piece) {
            sink.write(chunk).await?;
        }
        sink.finish().await?;
        Ok(out.0)
    }

    /// Produced by rclone v1.75.2 with password "password".
    #[tokio::test]
    async fn compatible_with_rclone() {
        let c = cipher(true);
        for (name, encrypted) in [
            ("a", "n1q525q8duc7lmkia8k5agaibs"),
            ("hello.txt", "cqk5s00ptd8rvarauufr0safk4"),
            (
                "Отчёт 2024.pdf",
                "1itvojj0ik8rh0tekbto2tjgii08o3vealhe5hb0g4befl4h2gvg",
            ),
            (
                "exactly16bytes!!",
                "beobc8aqufd2odc4vnosnuo56nvlaltqbtjarsajmq5o6rq2ittg",
            ),
        ] {
            assert_eq!(c.encrypt_file_name(name), encrypted);
            assert_eq!(c.decrypt_file_name(encrypted).as_deref(), Some(name));
        }
        let salted = cipher_with("password", "my salt", true);
        assert_eq!(salted.encrypt_file_name("a"), "9vfem29cgkroi100pv25biqs54");

        let file = hex::decode(
            "52434c4f4e45000088703558bf642e0f8aa2ddc930e0bdb20e93ab5ec30b0f08\
             2efd963e7ea41263227c15051fcd6ea23da6af993ab6",
        )
        .unwrap();
        assert_eq!(decrypt(&c, &file, 7).await.unwrap(), b"hello\n");
    }

    #[test]
    fn base32hex_round_trips() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            let text = base32hex_encode(data);
            assert_eq!(base32hex_decode(&text).unwrap(), data);
        }
        // RFC 4648 test vectors, lowercase and without padding.
        assert_eq!(base32hex_encode(b"foobar"), "cpnmuoj1e8");
        assert_eq!(base32hex_encode(b"fooba"), "cpnmuoj1");
        assert!(base32hex_decode("w").is_none());
    }

    #[test]
    fn names_round_trip() {
        let c = cipher(true);
        for name in ["a", "Отчёт 2024.pdf", "exactly16bytes!!", &"x".repeat(150)] {
            let encrypted = c.encrypt_file_name(name);
            assert_ne!(encrypted, name);
            assert_eq!(c.decrypt_file_name(&encrypted).as_deref(), Some(name));
        }
        assert_eq!(c.decrypt_file_name("not-encrypted.txt"), None);

        let plain = cipher(false);
        assert_eq!(plain.encrypt_file_name("a.txt"), "a.txt.bin");
        assert_eq!(
            plain.decrypt_file_name("a.txt.bin").as_deref(),
            Some("a.txt")
        );
        assert_eq!(plain.decrypt_file_name("a.txt"), None);
        assert_eq!(plain.encrypt_dir_name("dir"), "dir");
    }

    #[test]
    fn sizes_match_the_format() {
        for size in [0, 1, 100, 65535, 65536, 65537, 3 * 65536, 1_000_000] {
            assert_eq!(decrypted_size(encrypted_size(size)), Some(size), "{size}");
        }
        assert_eq!(encrypted_size(0), 32);
        assert_eq!(encrypted_size(1), 32 + 16 + 1);
        assert_eq!(decrypted_size(31), None);
        assert_eq!(decrypted_size(32 + 16), None);
    }

    #[test]
    fn nonce_addition_carries() {
        let mut n = [0xffu8, 0xff, 0, 0];
        add(&mut n, 1);
        assert_eq!(n, [0, 0, 1, 0]);
        let mut n = [0xf0u8, 0, 0, 0];
        add(&mut n, 0x1_0020);
        assert_eq!(n, [0x10, 1, 1, 0]);
    }

    #[tokio::test]
    async fn content_round_trips_from_any_offset() {
        let c = cipher(true);
        for size in [0usize, 1, 65536, 65537, 200_000] {
            let data: Vec<u8> = (0..size).map(|i| (i * 7 % 251) as u8).collect();
            let plain = BytesSource(data.clone().into());
            let source = EncryptedSource::new(&plain, c.clone()).unwrap();
            let full = read_all(&source, 0).await;
            assert_eq!(full.len() as u64, encrypted_size(size as u64));
            assert_eq!(source.len(), full.len() as u64);
            for piece in [1, 1000, 70_000] {
                assert_eq!(decrypt(&c, &full, piece).await.unwrap(), data);
            }
            // Resuming an upload reads the same bytes from the middle.
            for offset in [5, 32, 33, 32 + 65552, 32 + 65552 + 10, 100_000] {
                if offset <= full.len() {
                    assert_eq!(
                        read_all(&source, offset as u64).await,
                        full[offset..],
                        "{offset}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn bytes_match_the_stream_format() {
        let c = cipher(true);
        for size in [0usize, 1, 65536, 150_000] {
            let data: Vec<u8> = (0..size).map(|i| (i * 13 % 251) as u8).collect();
            let file = c.encrypt_bytes(&data).unwrap();
            assert_eq!(file.len() as u64, encrypted_size(size as u64));
            assert_eq!(decrypt(&c, &file, 1000).await.unwrap(), data);
            assert_eq!(c.decrypt_bytes(&file).unwrap(), data);
        }
        let file = c.encrypt_bytes(b"secret").unwrap();
        assert!(cipher_with("other", "", true).decrypt_bytes(&file).is_err());
        assert!(c.decrypt_bytes(b"short").is_err());
    }

    #[test]
    fn sealed_files_need_the_password() {
        let data = "аккаунты".as_bytes();
        let file = seal("pass", data).unwrap();
        assert_ne!(seal("pass", data).unwrap(), file, "the salt is random");
        assert_eq!(unseal("pass", &file).unwrap(), data);
        assert!(unseal("other", &file).is_err());
        assert!(unseal("pass", &file[..20]).is_err());
        assert!(unseal("pass", b"{\"accounts\": []}").is_err());
        assert!(seal("", data).is_err());
    }

    #[tokio::test]
    async fn decrypt_sink_survives_a_restart() {
        let c = cipher(true);
        let data = vec![9u8; 150_000];
        let plain = BytesSource(data.clone().into());
        let full = read_all(&EncryptedSource::new(&plain, c.clone()).unwrap(), 0).await;

        let mut out = MemorySink::default();
        let mut sink = DecryptSink::new(c.clone(), &mut out);
        sink.write(&full[..100_000]).await.unwrap();
        assert_eq!(sink.position(), 100_000);
        sink.reset().await.unwrap();
        sink.write(&full).await.unwrap();
        assert_eq!(sink.finish().await.unwrap(), 150_000);
        assert_eq!(out.0, data);
    }

    #[tokio::test]
    async fn wrong_password_is_detected() {
        let plain = BytesSource(b"secret".to_vec().into());
        let full = read_all(&EncryptedSource::new(&plain, cipher(true)).unwrap(), 0).await;
        let other = cipher_with("other", "", true);
        assert!(decrypt(&other, &full, 100).await.is_err());
        assert!(decrypt(
            &cipher(true),
            b"plain text that is long enough to be a header",
            100
        )
        .await
        .is_err());
        assert!(decrypt(&cipher(true), b"short", 100).await.is_err());
    }
}
