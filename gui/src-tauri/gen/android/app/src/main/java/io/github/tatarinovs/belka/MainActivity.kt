package io.github.tatarinovs.belka

import android.content.res.Configuration
import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    // Folder navigation is application state, not WebView history. Tauri
    // disables Wry's history handler, so forward Back explicitly to the UI.
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      private var pending = false

      override fun handleOnBackPressed() {
        if (pending) return
        pending = true
        webView.evaluateJavascript(
          "window.dispatchEvent(new Event('belka-back', { cancelable: true }))",
        ) { unhandled ->
          pending = false
          if (unhandled == "true") moveTaskToBack(true)
        }
      }
    })
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // The page is laid out between the system bars and above the keyboard,
    // on the background of its own toolbars.
    val content = findViewById<View>(android.R.id.content)
    val dark = (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
      Configuration.UI_MODE_NIGHT_YES
    content.setBackgroundColor(if (dark) 0xFF1E2126.toInt() else 0xFFFFFFFF.toInt())
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout(),
      )
      val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
      view.setPadding(bars.left, bars.top, bars.right, maxOf(bars.bottom, ime.bottom))
      WindowInsetsCompat.CONSUMED
    }
  }
}
