// The login flow, mirroring the GTK pages: greeter → homeserver → password.
// Native idioms, same places for the same buttons.
package io.github.steeb_k.commune.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState

private enum class LoginPage { Greeter, Homeserver, Password }

@Composable
fun LoginFlow(state: CommuneState) {
    var page by remember { mutableStateOf(LoginPage.Greeter) }
    var homeserver by remember { mutableStateOf("") }
    var useMatrixOrg by remember { mutableStateOf(true) }

    when (page) {
        LoginPage.Greeter -> Greeter(onLogIn = { page = LoginPage.Homeserver })

        LoginPage.Homeserver -> {
            BackHandler { page = LoginPage.Greeter }
            HomeserverPage(
                useMatrixOrg = useMatrixOrg,
                onUseMatrixOrg = { useMatrixOrg = it },
                homeserver = homeserver,
                onHomeserver = { homeserver = it },
                onNext = { page = LoginPage.Password },
            )
        }

        LoginPage.Password -> {
            BackHandler { page = LoginPage.Homeserver }
            PasswordPage(
                state = state,
                homeserver = if (useMatrixOrg) "https://matrix.org" else homeserver,
            )
        }
    }
}

/// One centered, clamped login column — the Adw.Clamp of every GTK login
/// page.
@Composable
private fun LoginColumn(content: @Composable () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Column(
            modifier = Modifier.widthIn(max = 400.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            content()
        }
    }
}

@Composable
private fun Greeter(onLogIn: () -> Unit) {
    LoginColumn {
        InitialsAvatar(identifier = "commune", name = "Commune", size = 96.dp)
        Spacer(Modifier.height(24.dp))
        Text(
            "Welcome to Commune",
            style = MaterialTheme.typography.headlineMedium,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(32.dp))
        Button(onClick = onLogIn, modifier = Modifier.widthIn(min = 260.dp)) {
            Text("Log In")
        }
        Spacer(Modifier.height(12.dp))
        OutlinedButton(
            onClick = {},
            enabled = false,
            modifier = Modifier.widthIn(min = 260.dp),
        ) {
            Text("Create Account")
        }
    }
}

@Composable
private fun HomeserverPage(
    useMatrixOrg: Boolean,
    onUseMatrixOrg: (Boolean) -> Unit,
    homeserver: String,
    onHomeserver: (String) -> Unit,
    onNext: () -> Unit,
) {
    LoginColumn {
        Text(
            "Choose a Homeserver",
            style = MaterialTheme.typography.headlineMedium,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(24.dp))

        androidx.compose.foundation.layout.Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.fillMaxWidth(),
        ) {
            RadioButton(selected = useMatrixOrg, onClick = { onUseMatrixOrg(true) })
            Text("matrix.org")
        }
        androidx.compose.foundation.layout.Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.fillMaxWidth(),
        ) {
            RadioButton(selected = !useMatrixOrg, onClick = { onUseMatrixOrg(false) })
            Text("Another Homeserver")
        }

        if (!useMatrixOrg) {
            OutlinedTextField(
                value = homeserver,
                onValueChange = onHomeserver,
                label = { Text("Homeserver URL") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
        }

        Spacer(Modifier.height(24.dp))
        Button(
            onClick = onNext,
            enabled = useMatrixOrg || homeserver.isNotBlank(),
            modifier = Modifier.widthIn(min = 260.dp),
        ) {
            Text("Next")
        }
    }
}

@Composable
private fun PasswordPage(state: CommuneState, homeserver: String) {
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }

    LoginColumn {
        Text(
            "Log In",
            style = MaterialTheme.typography.headlineMedium,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(8.dp))
        Text(homeserver, style = MaterialTheme.typography.bodyMedium)
        Spacer(Modifier.height(24.dp))

        OutlinedTextField(
            value = username,
            onValueChange = { username = it },
            label = { Text("Matrix Username") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(12.dp))
        OutlinedTextField(
            value = password,
            onValueChange = { password = it },
            label = { Text("Password") },
            singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            modifier = Modifier.fillMaxWidth(),
        )

        state.loginError?.let { error ->
            Spacer(Modifier.height(12.dp))
            Text(error, color = MaterialTheme.colorScheme.error)
        }

        Spacer(Modifier.height(24.dp))
        if (state.loginBusy) {
            CircularProgressIndicator(modifier = Modifier.size(32.dp))
        } else {
            Button(
                onClick = { state.login(homeserver, username, password) },
                enabled = username.isNotBlank() && password.isNotEmpty(),
                modifier = Modifier.widthIn(min = 260.dp),
            ) {
                Text("Next")
            }
        }
    }
}
