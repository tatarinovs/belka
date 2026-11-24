package io.github.tatarinovs.belka.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import io.github.tatarinovs.belka.core.Account
import io.github.tatarinovs.belka.core.AndroidPlatform
import io.github.tatarinovs.belka.core.BelkaCore
import io.github.tatarinovs.belka.core.CryptFolder
import io.github.tatarinovs.belka.core.FieldType
import io.github.tatarinovs.belka.core.OAuthApp
import io.github.tatarinovs.belka.core.OAuthService
import io.github.tatarinovs.belka.core.PROVIDERS
import io.github.tatarinovs.belka.core.ProviderKind
import io.github.tatarinovs.belka.core.UrlOpener
import io.github.tatarinovs.belka.core.buildProviderConfig
import io.github.tatarinovs.belka.core.missingProviderFields
import io.github.tatarinovs.belka.core.presetFields
import io.github.tatarinovs.belka.core.providerDef
import io.github.tatarinovs.belka.ui.theme.BelkaTheme
import kotlinx.coroutines.launch
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive

private data class OAuthHelp(val steps: String, val secretRequired: Boolean)

private val OAUTH_HELP = mapOf(
    OAuthService.GOOGLE to OAuthHelp(
        steps = "Google Cloud Console → APIs & Services: включите Google Drive API и создайте OAuth client ID типа «Desktop app».",
        secretRequired = true,
    ),
    OAuthService.YANDEX to OAuthHelp(
        steps = "oauth.yandex.ru → «Создать приложение»: платформа «Веб-сервисы», Redirect URI http://127.0.0.1:53682/, доступы Яндекс.Диска: чтение, запись и информация о Диске.",
        secretRequired = false,
    ),
)

@Composable
fun AccountDialog(
    account: Account?,
    accounts: List<Account>,
    onDismiss: () -> Unit,
    onSaved: (Account) -> Unit,
) {
    val colors = BelkaTheme.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    val editing = account != null
    val clouds = remember(accounts) { accounts.filter { it.kind != ProviderKind.CRYPT } }

    var kind by remember(account) { mutableStateOf(account?.kind ?: ProviderKind.YANDEX) }
    var name by remember(account) { mutableStateOf(account?.name.orEmpty()) }
    val stringValues = remember(account) { mutableStateMapOf<String, String>() }
    val boolValues = remember(account) { mutableStateMapOf<String, Boolean>() }
    var presetId by remember(account) { mutableStateOf("") }
    var busy by remember(account) { mutableStateOf(false) }
    var loading by remember(account) { mutableStateOf(false) }
    var waiting by remember(account) { mutableStateOf(false) }
    var error by remember(account) { mutableStateOf("") }
    var manual by remember(account) { mutableStateOf(account != null) }

    var oauthClientId by remember(account) { mutableStateOf("") }
    var oauthClientSecret by remember(account) { mutableStateOf("") }
    var hasBuiltin by remember(account) { mutableStateOf(false) }
    var useCustom by remember(account) { mutableStateOf(false) }

    var cryptBase by remember(account) { mutableStateOf(clouds.firstOrNull()?.id.orEmpty()) }
    var cryptRoot by remember(account) { mutableStateOf("/Encrypted") }
    var cryptPassword by remember(account) { mutableStateOf("") }
    var cryptRepeat by remember(account) { mutableStateOf("") }
    var cryptSalt by remember(account) { mutableStateOf("") }
    var cryptEncryptNames by remember(account) { mutableStateOf(true) }
    var showRcloneDetails by remember(account) { mutableStateOf(false) }

    var loadedConfigJson by remember(account) { mutableStateOf("") }

    val def = providerDef(kind)
    val preset = def.presets.firstOrNull { it.id == presetId }
    val fields = presetFields(def, preset)
    val service: OAuthService? = when (kind) {
        ProviderKind.GOOGLE -> OAuthService.GOOGLE
        ProviderKind.YANDEX -> OAuthService.YANDEX
        else -> null
    }
    val browserMode = service != null && !manual

    fun loadOAuthApp(svc: OAuthService) {
        oauthClientId = ""
        oauthClientSecret = ""
        hasBuiltin = false
        useCustom = false
        scope.launch {
            val apps = runCatching { BelkaCore.oauthApps(svc) }.getOrNull()
            val stillSvc = when (kind) {
                ProviderKind.GOOGLE -> OAuthService.GOOGLE
                ProviderKind.YANDEX -> OAuthService.YANDEX
                else -> null
            }
            if (stillSvc != svc) return@launch
            if (apps?.custom != null) {
                oauthClientId = apps.custom.client_id
                oauthClientSecret = apps.custom.client_secret
            }
            hasBuiltin = apps?.builtin ?: false
            useCustom = !hasBuiltin || apps?.custom != null
        }
    }

    LaunchedEffect(account) {
        if (account != null) {
            if (account.kind == ProviderKind.GOOGLE) loadOAuthApp(OAuthService.GOOGLE)
            if (account.kind == ProviderKind.YANDEX) loadOAuthApp(OAuthService.YANDEX)
            if (account.kind != ProviderKind.CRYPT) {
                loading = true
                try {
                    val config = BelkaCore.accountConfig(account.id)
                    if (config != null) {
                        val d = providerDef(account.kind)
                        for (f in d.fields) {
                            val el = config[f.key]?.jsonPrimitive ?: continue
                            if (f.type == FieldType.CHECKBOX) {
                                boolValues[f.key] = el.booleanOrNull ?: false
                            } else {
                                stringValues[f.key] = el.contentOrNull ?: el.toString()
                            }
                        }
                        loadedConfigJson = buildProviderConfig(d, stringValues, boolValues).toString()
                    }
                } catch (e: Exception) {
                    error = e.message ?: e.toString()
                } finally {
                    loading = false
                }
            }
        } else {
            loadOAuthApp(OAuthService.YANDEX)
        }
    }

    fun selectKind(next: ProviderKind) {
        kind = next
        stringValues.clear()
        boolValues.clear()
        presetId = ""
        error = ""
        manual = false
        if (next == ProviderKind.CRYPT) {
            cryptBase = clouds.firstOrNull()?.id.orEmpty()
            cryptRoot = "/Encrypted"
            cryptPassword = ""
            cryptRepeat = ""
            cryptSalt = ""
            cryptEncryptNames = true
        }
        when (next) {
            ProviderKind.GOOGLE -> loadOAuthApp(OAuthService.GOOGLE)
            ProviderKind.YANDEX -> loadOAuthApp(OAuthService.YANDEX)
            else -> {}
        }
    }

    fun selectPreset(id: String) {
        val prevPreset = def.presets.firstOrNull { it.id == presetId }
        val nextPreset = def.presets.firstOrNull { it.id == id }
        for ((k, v) in prevPreset?.values.orEmpty()) {
            if (stringValues[k] == v) stringValues.remove(k)
        }
        for ((k, v) in nextPreset?.values.orEmpty()) {
            stringValues[k] = v
        }
        presetId = id
    }

    fun signIn(svc: OAuthService) {
        val help = OAUTH_HELP.getValue(svc)
        if (useCustom && (oauthClientId.trim().isEmpty() || (help.secretRequired && oauthClientSecret.trim().isEmpty()))) {
            error = if (help.secretRequired) {
                "Укажите Client ID и Client secret OAuth-приложения."
            } else {
                "Укажите Client ID OAuth-приложения."
            }
            return
        }
        busy = true
        waiting = true
        error = ""
        scope.launch {
            try {
                val customApp = if (useCustom) OAuthApp(oauthClientId, oauthClientSecret) else null
                val opener = UrlOpener { url -> AndroidPlatform.openUrl(context, url) }
                val saved = BelkaCore.oauthSignIn(svc, customApp, name, account?.id, opener)
                onSaved(saved)
            } catch (e: Exception) {
                val text = e.message ?: e.toString()
                if (!text.contains("Вход отменён") && !text.contains("Sign-in cancelled")) {
                    error = text
                }
            } finally {
                busy = false
                waiting = false
            }
        }
    }

    fun addCrypt() {
        val base = clouds.firstOrNull { it.id == cryptBase }
        error = when {
            base == null -> "Выберите облако."
            cryptRoot.trim().isEmpty() -> "Укажите папку в облаке."
            cryptPassword.isEmpty() -> "Задайте пароль."
            cryptPassword != cryptRepeat -> "Пароли не совпадают."
            else -> ""
        }
        if (error.isNotEmpty() || base == null) return
        busy = true
        scope.launch {
            try {
                val root = cryptRoot.trim()
                val title = name.trim().ifEmpty { "${base.name} · $root 🔒" }
                val saved = BelkaCore.addCryptFolder(
                    title,
                    CryptFolder(
                        base = base.id,
                        root = root,
                        password = cryptPassword,
                        salt = cryptSalt,
                        encrypt_names = cryptEncryptNames,
                    ),
                )
                onSaved(saved)
            } catch (e: Exception) {
                error = e.message ?: e.toString()
            } finally {
                busy = false
            }
        }
    }

    fun submit() {
        if (busy || loading) return
        error = ""
        if (browserMode && service != null) {
            signIn(service)
            return
        }
        if (kind == ProviderKind.CRYPT && !editing) {
            addCrypt()
            return
        }
        if (kind != ProviderKind.CRYPT) {
            val missing = missingProviderFields(def, stringValues)
            if (missing.isNotEmpty()) {
                error = "Заполните: ${missing.joinToString(", ")}"
                return
            }
        }
        if (editing && name.trim().isEmpty()) {
            error = "Укажите название."
            return
        }
        busy = true
        scope.launch {
            try {
                val config = if (kind == ProviderKind.CRYPT) {
                    null
                } else {
                    buildProviderConfig(def, stringValues, boolValues)
                }
                val saved = if (account != null) {
                    val changed = config != null && config.toString() != loadedConfigJson
                    BelkaCore.updateAccount(account.id, name.trim(), if (changed) config else null)
                } else {
                    BelkaCore.addAccount(name.trim().ifEmpty { def.title }, config!!)
                }
                onSaved(saved)
            } catch (e: Exception) {
                error = e.message ?: e.toString()
            } finally {
                busy = false
            }
        }
    }

    fun close() {
        if (waiting) scope.launch { runCatching { BelkaCore.oauthCancel() } }
        onDismiss()
    }

    Dialog(
        onDismissRequest = { if (!busy || waiting) close() },
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .padding(8.dp)
                .widthIn(max = 520.dp),
            shape = RoundedCornerShape(16.dp),
            color = colors.surface,
            border = BorderStroke(1.dp, colors.border),
        ) {
            Column(
                modifier = Modifier
                    .verticalScroll(rememberScrollState())
                    .padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(
                    text = if (editing) "Настройки аккаунта" else "Новый аккаунт",
                    color = colors.text,
                    fontSize = 18.sp,
                    fontWeight = FontWeight.SemiBold,
                )

                if (editing) {
                    Text(
                        text = def.title,
                        color = colors.text,
                        fontSize = 15.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                } else {
                    BelkaDropdown(
                        label = "Тип",
                        selectedText = def.title,
                        options = PROVIDERS.map { it.kind to it.title },
                        enabled = !busy,
                        onSelect = ::selectKind,
                    )
                }

                if (waiting) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(vertical = 12.dp, horizontal = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(14.dp),
                    ) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(24.dp),
                            color = colors.accent,
                            strokeWidth = 3.dp,
                        )
                        Text(
                            text = "Откройте вкладку браузера и разрешите доступ к ${def.title}.\nОкно можно не закрывать: вход завершится автоматически.",
                            color = colors.muted,
                            fontSize = 14.sp,
                        )
                    }
                } else {
                    val helpText = when {
                        browserMode -> (if (editing) "Войдите заново, если доступ отозван или истёк: " else "Вход через браузер: ") +
                            "вы разрешите доступ на странице ${def.title}."
                        editing && kind == ProviderKind.CRYPT ->
                            "Можно изменить название. Пароль и папку шифрования изменить нельзя: для этого создайте новую зашифрованную папку."
                        else -> preset?.help ?: def.help
                    }
                    Text(text = helpText, color = colors.muted, fontSize = 14.sp)

                    if (def.presets.isNotEmpty() && !editing) {
                        BelkaDropdown(
                            label = "Сервис",
                            selectedText = preset?.title ?: "Выберите сервис",
                            options = def.presets.map { it.id to it.title },
                            enabled = !busy,
                            onSelect = ::selectPreset,
                        )
                    }

                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text("Название", color = colors.muted, fontSize = 12.5.sp)
                        BelkaInputField(
                            value = name,
                            onValueChange = { name = it },
                            placeholder = when {
                                editing -> ""
                                browserMode -> "По умолчанию — e-mail или логин"
                                else -> def.title
                            },
                            enabled = !busy,
                        )
                    }

                    if (kind == ProviderKind.CRYPT && !editing) {
                        if (clouds.isEmpty()) {
                            BelkaErrorBox("Сначала подключите облако, в котором будет лежать папка.")
                        } else {
                            BelkaDropdown(
                                label = "Облако *",
                                selectedText = clouds.firstOrNull { it.id == cryptBase }?.let {
                                    "${it.name} — ${providerDef(it.kind).title}"
                                }.orEmpty(),
                                options = clouds.map { it.id to "${it.name} — ${providerDef(it.kind).title}" },
                                enabled = !busy,
                                onSelect = { cryptBase = it },
                            )
                            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                Text("Папка в облаке *", color = colors.muted, fontSize = 12.5.sp)
                                BelkaInputField(
                                    value = cryptRoot,
                                    onValueChange = { cryptRoot = it },
                                    enabled = !busy,
                                )
                            }
                            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                Text("Пароль *", color = colors.muted, fontSize = 12.5.sp)
                                BelkaInputField(
                                    value = cryptPassword,
                                    onValueChange = { cryptPassword = it },
                                    isPassword = true,
                                    enabled = !busy,
                                )
                            }
                            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                Text("Пароль ещё раз *", color = colors.muted, fontSize = 12.5.sp)
                                BelkaInputField(
                                    value = cryptRepeat,
                                    onValueChange = { cryptRepeat = it },
                                    isPassword = true,
                                    enabled = !busy,
                                )
                            }
                            BelkaCheckboxRow(
                                checked = cryptEncryptNames,
                                onCheckedChange = { cryptEncryptNames = it },
                                label = "Шифровать имена файлов и папок",
                                enabled = !busy,
                            )
                            Text(
                                text = if (showRcloneDetails) "▾ Совместимость с rclone" else "▸ Совместимость с rclone",
                                color = colors.muted,
                                fontSize = 12.5.sp,
                                modifier = Modifier.clickable { showRcloneDetails = !showRcloneDetails },
                            )
                            if (showRcloneDetails) {
                                Text(
                                    text = "Файлы можно открыть через rclone: remote типа crypt на эту папку с тем же паролем, filename_encryption = ${if (cryptEncryptNames) "standard" else "off"}.",
                                    color = colors.muted,
                                    fontSize = 12.5.sp,
                                )
                                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    Text("Соль (password2 в rclone), если задана", color = colors.muted, fontSize = 12.sp)
                                    BelkaInputField(
                                        value = cryptSalt,
                                        onValueChange = { cryptSalt = it },
                                        isPassword = true,
                                        enabled = !busy,
                                    )
                                }
                            }
                            Box(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clip(RoundedCornerShape(8.dp))
                                    .background(colors.warningBg)
                                    .padding(horizontal = 10.dp, vertical = 8.dp),
                            ) {
                                Text(
                                    text = "Пароль невозможно восстановить: без него файлы не расшифровать ни Belka, ни кому-либо ещё. Сохраните его в надёжном месте.",
                                    color = colors.text,
                                    fontSize = 12.5.sp,
                                )
                            }
                        }
                    } else if (browserMode && service != null) {
                        val help = OAUTH_HELP.getValue(service)
                        Column(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clip(RoundedCornerShape(8.dp))
                                .border(1.dp, colors.border, RoundedCornerShape(8.dp))
                                .padding(10.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            if (hasBuiltin) {
                                BelkaCheckboxRow(
                                    checked = useCustom,
                                    onCheckedChange = { useCustom = it },
                                    label = "Своё OAuth-приложение",
                                    enabled = !busy,
                                )
                                if (!useCustom) {
                                    Text(
                                        text = "Используется встроенное приложение Belka, ничего настраивать не нужно.",
                                        color = colors.muted,
                                        fontSize = 12.5.sp,
                                    )
                                }
                            } else {
                                Text(
                                    text = "В этой сборке нет встроенного приложения, укажите своё.",
                                    color = colors.muted,
                                    fontSize = 12.5.sp,
                                )
                            }
                            if (useCustom) {
                                Text(text = help.steps, color = colors.muted, fontSize = 12.5.sp)
                                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    Text("Client ID *", color = colors.muted, fontSize = 12.sp)
                                    BelkaInputField(
                                        value = oauthClientId,
                                        onValueChange = { oauthClientId = it },
                                        enabled = !busy,
                                    )
                                }
                                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    Text(
                                        text = "Client secret${if (help.secretRequired) " *" else " (не обязателен)"}",
                                        color = colors.muted,
                                        fontSize = 12.sp,
                                    )
                                    BelkaInputField(
                                        value = oauthClientSecret,
                                        onValueChange = { oauthClientSecret = it },
                                        isPassword = true,
                                        enabled = !busy,
                                    )
                                }
                            }
                        }
                    } else if (kind != ProviderKind.CRYPT) {
                        fields.forEach { field ->
                            if (field.type == FieldType.CHECKBOX) {
                                BelkaCheckboxRow(
                                    checked = boolValues[field.key] == true,
                                    onCheckedChange = { boolValues[field.key] = it },
                                    label = field.label,
                                    enabled = !busy,
                                )
                            } else {
                                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    Text(
                                        text = "${field.label}${if (field.optional) "" else " *"}",
                                        color = colors.muted,
                                        fontSize = 12.5.sp,
                                    )
                                    BelkaInputField(
                                        value = stringValues[field.key].orEmpty(),
                                        onValueChange = { stringValues[field.key] = it },
                                        placeholder = field.placeholder,
                                        isPassword = field.type == FieldType.PASSWORD,
                                        keyboardType = when (field.type) {
                                            FieldType.URL -> KeyboardType.Uri
                                            FieldType.NUMBER -> KeyboardType.Number
                                            else -> KeyboardType.Text
                                        },
                                        enabled = !busy && !loading,
                                    )
                                    if (field.hint.isNotEmpty()) {
                                        Text(text = field.hint, color = colors.muted, fontSize = 12.sp)
                                    }
                                }
                            }
                        }
                    }

                    if (service != null) {
                        Text(
                            text = if (manual) {
                                if (editing) "Войти заново через браузер" else "Войти через браузер"
                            } else {
                                "Ввести токен вручную"
                            },
                            color = colors.accent,
                            fontSize = 14.sp,
                            modifier = Modifier.clickable(enabled = !busy) {
                                manual = !manual
                                error = ""
                            },
                        )
                    }
                }

                if (error.isNotEmpty()) {
                    BelkaErrorBox(error)
                }

                Text(
                    text = "${if (kind == ProviderKind.CRYPT) "Пароль хранится" else "Данные входа хранятся"} в закрытой папке приложения.",
                    color = colors.muted,
                    fontSize = 12.5.sp,
                )

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
                ) {
                    if (waiting) {
                        BelkaSecondaryButton(onClick = {
                            scope.launch { runCatching { BelkaCore.oauthCancel() } }
                        }) {
                            Text("Отменить вход")
                        }
                    } else {
                        BelkaSecondaryButton(enabled = !busy, onClick = ::close) {
                            Text("Отмена")
                        }
                        BelkaPrimaryButton(enabled = !busy, onClick = ::submit) {
                            Text(
                                when {
                                    busy -> "Проверка…"
                                    browserMode -> "Войти через браузер"
                                    editing -> "Сохранить"
                                    kind == ProviderKind.CRYPT -> "Создать"
                                    else -> "Подключить"
                                },
                            )
                        }
                    }
                }
            }
        }
    }
}
