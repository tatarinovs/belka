package io.github.tatarinovs.belka

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import io.github.tatarinovs.belka.core.AndroidPlatform
import io.github.tatarinovs.belka.ui.BelkaViewModel
import io.github.tatarinovs.belka.ui.MainScreen
import io.github.tatarinovs.belka.ui.theme.BelkaTheme

class MainActivity : ComponentActivity() {
    private val viewModel: BelkaViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        AndroidPlatform.attachActivity(this)

        setContent {
            BelkaTheme {
                BackHandler {
                    if (!viewModel.onSystemBack()) {
                        moveTaskToBack(true)
                    }
                }
                MainScreen(vm = viewModel)
            }
        }
    }

    override fun onResume() {
        super.onResume()
        AndroidPlatform.attachActivity(this)
        viewModel.onResume()
    }

    override fun onDestroy() {
        AndroidPlatform.detachActivity(this)
        super.onDestroy()
    }
}
