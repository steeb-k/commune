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

private enum class LoginPage { Greeter, Homeserver, Method, Password, Register, Reset }

@Composable
fun LoginFlow(state: CommuneState) {
    var page by remember { mutableStateOf(LoginPage.Greeter) }
    var homeserver by remember { mutableStateOf("") }
    var useMatrixOrg by remember { mutableStateOf(true) }
    var creatingAccount by remember { mutableStateOf(false) }

    val resolvedHomeserver = if (useMatrixOrg) {
        "https://matrix.org"
    } else {
        homeserver.trim().let {
            if (it.contains("://")) it else "https://" + it
        }
    }

    when (page) {
        LoginPage.Greeter -> Greeter(
            onLogIn = {
                creatingAccount = false
                page = LoginPage.Homeserver
            },
            onCreateAccount = {
                creatingAccount = true
                page = LoginPage.Homeserver
            },
        )

        LoginPage.Homeserver -> {
            BackHandler { page = LoginPage.Greeter }
            HomeserverPage(
                state = state,
                useMatrixOrg = useMatrixOrg,
                onUseMatrixOrg = { useMatrixOrg = it },
                homeserver = homeserver,
                onHomeserver = { homeserver = it },
                onNext = {
                    // Ask the homeserver what it offers before choosing a
                    // page, as the application's discovery does.
                    state.discoverLogin(resolvedHomeserver) { ok ->
                        if (!ok) return@discoverLogin
                        val methods = state.loginMethods
                        page = when {
                            creatingAccount -> LoginPage.Register
                            methods?.supportsOauth == true -> {
                                state.startBrowserLogin(oauth = true)
                                LoginPage.Method
                            }
                            methods?.supportsPassword == true -> LoginPage.Method
                            methods?.supportsSso == true -> {
                                state.startBrowserLogin(oauth = false)
                                LoginPage.Method
                            }
                            else -> LoginPage.Method
                        }
                    }
                },
            )
        }

        LoginPage.Method -> {
            BackHandler { page = LoginPage.Homeserver }
            MethodPage(
                state = state,
                onPassword = { page = LoginPage.Password },
                onSso = { state.startBrowserLogin(oauth = false) },
                onOauth = { state.startBrowserLogin(oauth = true) },
                onForgot = { page = LoginPage.Reset },
            )
        }

        LoginPage.Password -> {
            BackHandler { page = LoginPage.Method }
            PasswordPage(state = state, homeserver = resolvedHomeserver)
        }

        LoginPage.Register -> {
            BackHandler { page = LoginPage.Homeserver }
            RegisterPage(state = state, homeserver = resolvedHomeserver)
        }

        LoginPage.Reset -> {
            BackHandler { page = LoginPage.Method }
            ResetPasswordPage(state = state, onDone = { page = LoginPage.Method })
        }
    }
}

/// The choice of login method, once discovery answered.
@Composable
private fun MethodPage(
    state: CommuneState,
    onPassword: () -> Unit,
    onSso: () -> Unit,
    onOauth: () -> Unit,
    onForgot: () -> Unit,
) {
    val methods = state.loginMethods

    LoginColumn {
        Text(
            "Log In",
            style = MaterialTheme.typography.headlineMedium,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(8.dp))
        Text(
            methods?.homeserverUrl.orEmpty(),
            style = MaterialTheme.typography.bodyMedium,
        )
        Spacer(Modifier.height(24.dp))

        if (methods?.supportsOauth == true) {
            Text(
                "This homeserver logs you in through its own website.",
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(16.dp))
            Button(onClick = onOauth, modifier = Modifier.widthIn(min = 260.dp)) {
                Text("Continue in Browser")
            }
        } else {
            if (methods?.supportsPassword == true) {
                Button(onClick = onPassword, modifier = Modifier.widthIn(min = 260.dp)) {
                    Text("Log In with Password")
                }
                Spacer(Modifier.height(12.dp))
            }
            if (methods?.supportsSso == true) {
                OutlinedButton(onClick = onSso, modifier = Modifier.widthIn(min = 260.dp)) {
                    Text("Log In with Single Sign-On")
                }
                Spacer(Modifier.height(12.dp))
            }
            androidx.compose.material3.TextButton(onClick = onForgot) {
                Text("Forgot Password?")
            }
        }

        state.loginError?.let { error ->
            Spacer(Modifier.height(12.dp))
            Text(error, color = MaterialTheme.colorScheme.error, textAlign = TextAlign.Center)
        }
        if (state.loginBusy) {
            Spacer(Modifier.height(16.dp))
            LoadingRing(modifier = Modifier.size(56.dp))
        }
    }
}

/// Create an account: the register page, to the extent a headless
/// client can answer the homeserver's steps.
@Composable
private fun RegisterPage(state: CommuneState, homeserver: String) {
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var confirm by remember { mutableStateOf("") }

    LoginColumn {
        Text(
            "Create Account",
            style = MaterialTheme.typography.headlineMedium,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(8.dp))
        Text(homeserver, style = MaterialTheme.typography.bodyMedium)
        Spacer(Modifier.height(24.dp))

        OutlinedTextField(
            value = username,
            onValueChange = { username = it },
            label = { Text("Username") },
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
        Spacer(Modifier.height(12.dp))
        OutlinedTextField(
            value = confirm,
            onValueChange = { confirm = it },
            label = { Text("Confirm Password") },
            singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            isError = confirm.isNotEmpty() && confirm != password,
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(8.dp))
        Text(
            "Creating an account accepts the homeserver's terms of service.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )

        state.loginError?.let { error ->
            Spacer(Modifier.height(12.dp))
            Text(error, color = MaterialTheme.colorScheme.error, textAlign = TextAlign.Center)
        }

        Spacer(Modifier.height(24.dp))
        if (state.loginBusy) {
            LoadingRing(modifier = Modifier.size(56.dp))
        } else {
            Button(
                onClick = { state.register(username.trim(), password) },
                enabled = username.isNotBlank() && password.isNotEmpty() &&
                    confirm == password,
                modifier = Modifier.widthIn(min = 260.dp),
            ) {
                Text("Create Account")
            }
        }
    }
}

/// The email-token password reset: ask for the email, then set the new
/// password once the link was opened.
@Composable
private fun ResetPasswordPage(state: CommuneState, onDone: () -> Unit) {
    var email by remember { mutableStateOf("") }
    var newPassword by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    val emailSent = state.resetHandle != null

    LoginColumn {
        Text(
            "Reset Password",
            style = MaterialTheme.typography.headlineMedium,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(24.dp))

        if (!emailSent) {
            OutlinedTextField(
                value = email,
                onValueChange = { email = it },
                label = { Text("Email Address") },
                singleLine = true,
                keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                    keyboardType = androidx.compose.ui.text.input.KeyboardType.Email,
                ),
                modifier = Modifier.fillMaxWidth(),
            )
        } else {
            Text(
                "Open the link sent to $email, then set a new password here.",
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(12.dp))
            OutlinedTextField(
                value = newPassword,
                onValueChange = { newPassword = it },
                label = { Text("New Password") },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                modifier = Modifier.fillMaxWidth(),
            )
        }

        error?.let {
            Spacer(Modifier.height(12.dp))
            Text(it, color = MaterialTheme.colorScheme.error, textAlign = TextAlign.Center)
        }

        Spacer(Modifier.height(24.dp))
        if (busy) {
            LoadingRing(modifier = Modifier.size(56.dp))
        } else if (!emailSent) {
            Button(
                onClick = {
                    busy = true
                    error = null
                    state.requestPasswordReset(email.trim()) { failure ->
                        busy = false
                        error = failure
                    }
                },
                enabled = email.contains("@"),
                modifier = Modifier.widthIn(min = 260.dp),
            ) {
                Text("Send Reset Email")
            }
        } else {
            Button(
                onClick = {
                    busy = true
                    error = null
                    state.resetPassword(newPassword) { failure ->
                        busy = false
                        if (failure == null) onDone() else error = failure
                    }
                },
                enabled = newPassword.isNotEmpty(),
                modifier = Modifier.widthIn(min = 260.dp),
            ) {
                Text("Set New Password")
            }
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
private fun Greeter(onLogIn: () -> Unit, onCreateAccount: () -> Unit) {
    LoginColumn {
        androidx.compose.material3.Icon(
            androidx.compose.ui.res.painterResource(
                io.github.steeb_k.commune.R.drawable.ic_app_symbolic
            ),
            contentDescription = null,
            tint = MaterialTheme.colorScheme.primary,
            modifier = Modifier.size(96.dp),
        )
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
            onClick = onCreateAccount,
            modifier = Modifier.widthIn(min = 260.dp),
        ) {
            Text("Create Account")
        }
    }
}

@Composable
private fun HomeserverPage(
    state: CommuneState,
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
                placeholder = { Text("https://example.org") },
                singleLine = true,
                keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                    keyboardType = androidx.compose.ui.text.input.KeyboardType.Uri,
                ),
                supportingText = if (
                    homeserver.isNotBlank() && !homeserver.contains("://")
                ) {
                    { Text("Will connect to https://${homeserver.trim()}") }
                } else {
                    null
                },
                modifier = Modifier.fillMaxWidth(),
            )
        }

        state.loginError?.let { error ->
            Spacer(Modifier.height(12.dp))
            Text(error, color = MaterialTheme.colorScheme.error, textAlign = TextAlign.Center)
        }

        Spacer(Modifier.height(24.dp))
        if (state.loginBusy) {
            LoadingRing(modifier = Modifier.size(56.dp))
        } else {
            Button(
                onClick = onNext,
                enabled = useMatrixOrg || homeserver.isNotBlank(),
                modifier = Modifier.widthIn(min = 260.dp),
            ) {
                Text("Next")
            }
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
            LoadingRing(modifier = Modifier.size(56.dp))
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
