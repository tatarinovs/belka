//! JNI bridge for the native Android Kotlin app (`io.github.tatarinovs.belka`).

use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use belka_app::error::{translate, AppResult};
use belka_app::local::android_places;
use belka_app::state::RemoteItem;
use belka_app::transfers::TransferFilter;
use belka_app::{AppCore, CloudItem, ShareParams};
use belka_core::oauth::{OAuthApp, Service};
use belka_core::ProviderConfig;
use belka_store::accounts::{AccountStore, CryptFolder};
use belka_store::secrets::Secrets;
use belka_store::tasks::{self, Task, TaskStore};
use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::jstring;
use jni::JNIEnv;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Once, OnceLock};
use tokio::runtime::Runtime;

static LOG_INIT: Once = Once::new();
static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static CORE: OnceLock<Arc<AppCore>> = OnceLock::new();

fn init_logging() {
    LOG_INIT.call_once(|| {
        let filter = tracing_subscriber::filter::Targets::new()
            .with_default(tracing_subscriber::filter::LevelFilter::WARN)
            .with_target("belka_core", tracing_subscriber::filter::LevelFilter::INFO)
            .with_target("belka_store", tracing_subscriber::filter::LevelFilter::INFO)
            .with_target("belka_app", tracing_subscriber::filter::LevelFilter::INFO)
            .with_target("belka_android", tracing_subscriber::filter::LevelFilter::INFO);
        use tracing_subscriber::prelude::*;
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing_subscriber::filter::LevelFilter::TRACE)
            .with_target(false)
            .finish()
            .with(filter)
            .try_init();
    });
}

fn runtime() -> Result<&'static Runtime> {
    if let Some(rt) = RUNTIME.get() {
        return Ok(rt);
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start async runtime")?;
    let _ = RUNTIME.set(rt);
    Ok(RUNTIME.get().expect("runtime initialized"))
}

fn core() -> Result<&'static Arc<AppCore>> {
    CORE.get()
        .context("Belka core is not initialized; call init() first")
}

fn jstring_to_string(env: &mut JNIEnv, value: &JString) -> String {
    env.get_string(value).map(Into::into).unwrap_or_default()
}

fn to_jstring(env: &mut JNIEnv, value: &str) -> jstring {
    env.new_string(value)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

fn ok_json<T: Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(v) => json!({ "ok": v }).to_string(),
        Err(err) => err_json(&format!("Serialization error: {err}")),
    }
}

fn err_json(msg: &str) -> String {
    json!({ "err": msg }).to_string()
}

fn wrap<T: Serialize>(res: AppResult<T>) -> String {
    match res {
        Ok(val) => ok_json(val),
        Err(err) => err_json(err.message()),
    }
}

fn parse_args<T: for<'de> Deserialize<'de>>(args_json: &str) -> Result<T> {
    serde_json::from_str(args_json).context("Invalid arguments")
}

fn dispatch(method: &str, args_json: &str) -> String {
    let core = match core() {
        Ok(c) => c,
        Err(err) => return err_json(&translate(&format!("{err:#}"))),
    };
    let rt = match runtime() {
        Ok(rt) => rt,
        Err(err) => return err_json(&translate(&format!("{err:#}"))),
    };

    match method {
        // ---- accounts ----
        "list_accounts" => wrap(core.list_accounts()),
        "add_account" => {
            #[derive(Deserialize)]
            struct Args {
                name: String,
                config: ProviderConfig,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.add_account(&a.name, a.config))),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "oauth_apps" => {
            #[derive(Deserialize)]
            struct Args {
                service: Service,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => ok_json(core.oauth_apps(a.service)),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "oauth_cancel" => {
            core.oauth_cancel();
            ok_json(Value::Null)
        }
        "add_crypt_folder" => {
            #[derive(Deserialize)]
            struct Args {
                name: String,
                folder: CryptFolder,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.add_crypt_folder(&a.name, a.folder))),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "account_config" => {
            #[derive(Deserialize)]
            struct Args {
                id: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(core.account_config(&a.id)),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "update_account" => {
            #[derive(Deserialize)]
            struct Args {
                id: String,
                name: String,
                config: Option<ProviderConfig>,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.update_account(&a.id, &a.name, a.config))),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "reorder_accounts" => {
            #[derive(Deserialize)]
            struct Args {
                ids: Vec<String>,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(core.reorder_accounts(&a.ids).map(|()| Value::Null)),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "remove_account" => {
            #[derive(Deserialize)]
            struct Args {
                id: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(core.remove_account(&a.id).map(|()| Value::Null)),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "export_accounts_bytes" => {
            #[derive(Deserialize)]
            struct Args {
                password: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => match rt.block_on(core.export_accounts_bytes(a.password)) {
                    Ok((count, bytes)) => ok_json(json!({
                        "count": count,
                        "data": BASE64.encode(bytes),
                    })),
                    Err(e) => err_json(e.message()),
                },
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "import_accounts_bytes" => {
            #[derive(Deserialize)]
            struct Args {
                data: String,
                password: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => match BASE64.decode(a.data) {
                    Ok(bytes) => wrap(rt.block_on(core.import_accounts_bytes(bytes, a.password))),
                    Err(e) => err_json(&format!("Invalid export data: {e}")),
                },
                Err(e) => err_json(&format!("{e:#}")),
            }
        }

        // ---- cloud ----
        "list_dir" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                account_id: String,
                path: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.list_dir(&a.account_id, &a.path))),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "cloud_quota" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                account_id: String,
                path: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.cloud_quota(&a.account_id, &a.path))),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "cloud_mkdir" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                account_id: String,
                parent: String,
                name: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.cloud_mkdir(&a.account_id, &a.parent, &a.name))),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "cloud_rename" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                account_id: String,
                path: String,
                new_name: String,
                is_dir: bool,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.cloud_rename(
                    &a.account_id,
                    &a.path,
                    &a.new_name,
                    a.is_dir,
                ))),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "cloud_delete" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                account_id: String,
                items: Vec<CloudItem>,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(
                    rt.block_on(core.cloud_delete(&a.account_id, &a.items))
                        .map(|()| Value::Null),
                ),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "cloud_share" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                account_id: String,
                items: Vec<CloudItem>,
                options: Option<ShareParams>,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.cloud_share(&a.account_id, &a.items, a.options))),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "cloud_unshare" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                account_id: String,
                items: Vec<CloudItem>,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(
                    rt.block_on(core.cloud_unshare(&a.account_id, &a.items))
                        .map(|()| Value::Null),
                ),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "preview_file" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                account_id: String,
                path: String,
                size: u64,
                modified: Option<String>,
                text: bool,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.preview_file(
                    &a.account_id,
                    &a.path,
                    a.size,
                    a.modified.as_deref(),
                    a.text,
                ))),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "preview_progress" => ok_json(core.preview_progress()),
        "preview_cancel" => {
            core.preview_cancel();
            ok_json(Value::Null)
        }
        "stream_open" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                account_id: String,
                path: String,
                size: u64,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.stream_open(&a.account_id, &a.path, a.size))),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "stream_open_player" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                account_id: String,
                path: String,
                size: u64,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => match rt.block_on(core.stream_open_player(&a.account_id, &a.path, a.size))
                {
                    Ok((url, mime, name)) => ok_json(json!({
                        "url": url,
                        "mime": mime,
                        "name": name,
                    })),
                    Err(e) => err_json(e.message()),
                },
                Err(e) => err_json(&format!("{e:#}")),
            }
        }

        // ---- local & transfers ----
        "download" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                account_id: String,
                items: Vec<RemoteItem>,
                local_dir: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.download(&a.account_id, a.items, &a.local_dir))),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "upload" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                account_id: String,
                local_paths: Vec<String>,
                remote_dir: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.upload(&a.account_id, a.local_paths, &a.remote_dir))),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "cloud_copy" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                account_id: String,
                items: Vec<RemoteItem>,
                target_account_id: String,
                target_dir: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.cloud_copy(
                    &a.account_id,
                    a.items,
                    &a.target_account_id,
                    &a.target_dir,
                ))),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "local_conflicts" => {
            #[derive(Deserialize)]
            struct Args {
                dir: String,
                names: Vec<String>,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => ok_json(core.local_conflicts(&a.dir, a.names)),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "list_local" => {
            #[derive(Deserialize)]
            struct Args {
                path: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.list_local(a.path, android_places()))),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "local_mkdir" => {
            #[derive(Deserialize)]
            struct Args {
                parent: String,
                name: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(core.local_mkdir(&a.parent, &a.name)),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "local_rename" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                path: String,
                new_name: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(core.local_rename(&a.path, &a.new_name)),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "local_delete" => {
            #[derive(Deserialize)]
            struct Args {
                paths: Vec<String>,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(rt.block_on(core.local_delete(a.paths)).map(|()| Value::Null)),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "list_transfers" => {
            #[derive(Deserialize)]
            struct Args {
                filter: Option<TransferFilter>,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => ok_json(core.list_transfers(a.filter)),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "cancel_transfer" => {
            #[derive(Deserialize)]
            struct Args {
                id: u64,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => {
                    core.cancel_transfer(a.id);
                    ok_json(Value::Null)
                }
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "cancel_all_transfers" => {
            core.cancel_all_transfers();
            ok_json(Value::Null)
        }
        "retry_transfer" => {
            #[derive(Deserialize)]
            struct Args {
                id: u64,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(core.retry_transfer(a.id).map(|()| Value::Null)),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "retry_failed_transfers" => ok_json(core.retry_failed_transfers()),
        "clear_transfers" => {
            core.clear_transfers();
            ok_json(Value::Null)
        }
        "transfer_activity" => ok_json(core.transfer_activity()),

        // ---- backups & updates ----
        "backup_tasks" => wrap(core.backup_tasks()),
        "backup_list" => wrap(core.backup_list()),
        "backup_save" => {
            #[derive(Deserialize)]
            struct Args {
                task: Task,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(core.backup_save(a.task)),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "backup_remove" => {
            #[derive(Deserialize)]
            struct Args {
                id: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => wrap(core.backup_remove(&a.id).map(|()| Value::Null)),
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "backup_cancel" => {
            #[derive(Deserialize)]
            struct Args {
                id: String,
            }
            match parse_args::<Args>(args_json) {
                Ok(a) => {
                    core.backup_cancel(&a.id);
                    ok_json(Value::Null)
                }
                Err(e) => err_json(&format!("{e:#}")),
            }
        }
        "check_update" => wrap(rt.block_on(belka_app::state::check_update(env!(
            "CARGO_PKG_VERSION"
        )))),
        _ => err_json(&format!("Unknown method '{method}'")),
    }
}

#[no_mangle]
pub extern "system" fn Java_io_github_tatarinovs_belka_core_BelkaNative_init<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    config_dir: JString<'l>,
    cache_dir: JString<'l>,
) {
    init_logging();
    let config_dir = jstring_to_string(&mut env, &config_dir);
    let cache_dir = jstring_to_string(&mut env, &cache_dir);
    let _ = CORE.get_or_init(|| {
        let config = Path::new(&config_dir);
        let cache = Path::new(&cache_dir);
        let secrets = Secrets::File(config.join("secrets.json"));
        Arc::new(AppCore::new(config, cache, secrets))
    });
}

#[no_mangle]
pub extern "system" fn Java_io_github_tatarinovs_belka_core_BelkaNative_call<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    method: JString<'l>,
    args_json: JString<'l>,
) -> jstring {
    let method = jstring_to_string(&mut env, &method);
    let args_json = jstring_to_string(&mut env, &args_json);
    let result = dispatch(&method, &args_json);
    to_jstring(&mut env, &result)
}

#[no_mangle]
pub extern "system" fn Java_io_github_tatarinovs_belka_core_BelkaNative_oauthSignIn<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    args_json: JString<'l>,
    opener: JObject<'l>,
) -> jstring {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        service: Service,
        custom_app: Option<OAuthApp>,
        name: String,
        account_id: Option<String>,
    }

    let args_json = jstring_to_string(&mut env, &args_json);
    let result = (|| -> Result<String> {
        let core = core()?;
        let rt = runtime()?;
        let args: Args = parse_args(&args_json)?;
        let jvm = env.get_java_vm().context("Missing JVM handle")?;
        let opener_ref = env
            .new_global_ref(opener)
            .context("Failed to retain UrlOpener")?;
        let open_url = move |url: &str| -> Result<()> {
            let mut env = jvm
                .attach_current_thread()
                .context("Failed to attach JVM thread")?;
            let jurl = env.new_string(url).context("Failed to allocate URL string")?;
            env.call_method(
                opener_ref.as_obj(),
                "openUrl",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&jurl)],
            )
            .context("Failed to open browser for sign-in")?;
            Ok(())
        };
        Ok(wrap(rt.block_on(core.oauth_sign_in(
            args.service,
            args.custom_app,
            &args.name,
            args.account_id.as_deref(),
            open_url,
        ))))
    })();

    let out = match result {
        Ok(json) => json,
        Err(err) => err_json(&translate(&format!("{err:#}"))),
    };
    to_jstring(&mut env, &out)
}

#[no_mangle]
pub extern "system" fn Java_io_github_tatarinovs_belka_core_BelkaNative_backupRun<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    dir: JString<'l>,
    id: JString<'l>,
) -> jstring {
    init_logging();
    let (dir, id) = (
        jstring_to_string(&mut env, &dir),
        jstring_to_string(&mut env, &id),
    );
    if tasks::is_running(&id) {
        return to_jstring(&mut env, r#"{"busy":true}"#);
    }
    let dir = Path::new(&dir);
    let accounts = AccountStore::new(dir, Secrets::File(dir.join("secrets.json")));
    let json = match tasks::run_blocking(&TaskStore::new(dir), &accounts, &id) {
        Ok(mut run) => {
            run.result.error = run.result.error.as_deref().map(translate);
            let mut value = serde_json::to_value(&run.result).unwrap_or_default();
            value["freed_paths"] = json!(run.freed);
            value.to_string()
        }
        Err(err) if err.is::<tasks::AlreadyRunning>() => r#"{"busy":true}"#.to_string(),
        Err(err) => json!({ "error": translate(&format!("{err:#}")) }).to_string(),
    };
    to_jstring(&mut env, &json)
}

#[no_mangle]
pub extern "system" fn Java_io_github_tatarinovs_belka_core_BelkaNative_backupProgress<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    id: JString<'l>,
) -> jstring {
    let id = jstring_to_string(&mut env, &id);
    let json = tasks::live(&id)
        .and_then(|live| serde_json::to_string(&live).ok())
        .unwrap_or_default();
    to_jstring(&mut env, &json)
}

#[no_mangle]
pub extern "system" fn Java_io_github_tatarinovs_belka_core_BelkaNative_backupCancel<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    id: JString<'l>,
) {
    let id = jstring_to_string(&mut env, &id);
    tasks::cancel(&id);
}
