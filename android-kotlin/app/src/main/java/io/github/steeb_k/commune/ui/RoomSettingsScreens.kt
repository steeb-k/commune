// The room-settings subpages and dialogs under room details: avatar,
// join rule, history visibility, addresses, server ACL, permissions and
// the room upgrade — the GTK room_details subpages, Android-shaped.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiAddressAction
import io.github.steeb_k.commune.core.FfiHistoryVisibility
import io.github.steeb_k.commune.core.FfiJoinRuleValue
import io.github.steeb_k.commune.core.FfiPowerLevelsMatrix
import io.github.steeb_k.commune.core.FfiRoomCategory

/// Change or remove the room's picture.
@Composable
internal fun RoomAvatarDialog(state: CommuneState, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Room Avatar") },
        text = {
            Column {
                TextButton(onClick = {
                    state.pickRoomAvatar?.invoke()
                    onDismiss()
                }) { Text("Choose a New Picture") }
                TextButton(onClick = {
                    state.removeRoomAvatar()
                    onDismiss()
                }) { Text("Remove Avatar", color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

/// The join rule: the GTK subpage's choice — public, invite, or the
/// members of a space — with knocking layered on where the room's
/// version takes it.
@Composable
internal fun JoinRuleDialog(state: CommuneState, onDismiss: () -> Unit) {
    val info = state.joinRuleInfo
    if (info == null) {
        AlertDialog(
            onDismissRequest = onDismiss,
            title = { Text("Who Can Join") },
            text = { Text("Loading…") },
            confirmButton = { TextButton(onClick = onDismiss) { Text("Close") } },
        )
        return
    }

    var selection by remember {
        mutableStateOf(
            when (info.value) {
                FfiJoinRuleValue.PUBLIC -> FfiJoinRuleValue.PUBLIC
                FfiJoinRuleValue.RESTRICTED, FfiJoinRuleValue.KNOCK_RESTRICTED ->
                    FfiJoinRuleValue.RESTRICTED
                else -> FfiJoinRuleValue.INVITE
            }
        )
    }
    var knock by remember {
        mutableStateOf(
            info.value == FfiJoinRuleValue.KNOCK ||
                info.value == FfiJoinRuleValue.KNOCK_RESTRICTED
        )
    }
    val spaces = state.rooms.filter { it.category == FfiRoomCategory.SPACE }
    var spaceId by remember { mutableStateOf(info.allowRoomIds.firstOrNull()) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }

    // Knocking applies to the invite rule, and to the membership rule
    // only where the room's version has knock_restricted.
    val knockApplies = info.supportsKnock &&
        (
            selection == FfiJoinRuleValue.INVITE ||
                (selection == FfiJoinRuleValue.RESTRICTED && info.supportsKnockRestricted)
            )

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Who Can Join") },
        text = {
            Column {
                @Composable
                fun choice(value: FfiJoinRuleValue, label: String) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable(enabled = info.canChange) { selection = value },
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(
                            selected = selection == value,
                            onClick = { selection = value },
                            enabled = info.canChange,
                        )
                        Text(label, style = MaterialTheme.typography.bodyLarge)
                    }
                }
                choice(FfiJoinRuleValue.INVITE, "Only invited people")
                choice(FfiJoinRuleValue.PUBLIC, "Anyone")
                if (info.supportsRestricted) {
                    choice(FfiJoinRuleValue.RESTRICTED, "Members of a space")
                }
                if (selection == FfiJoinRuleValue.RESTRICTED) {
                    for (space in spaces) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable(enabled = info.canChange) {
                                    spaceId = space.roomId
                                }
                                .padding(start = 24.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(
                                selected = spaceId == space.roomId,
                                onClick = { spaceId = space.roomId },
                                enabled = info.canChange,
                            )
                            Text(roomName(space), style = MaterialTheme.typography.bodyMedium)
                        }
                    }
                    if (spaces.isEmpty()) {
                        Text(
                            "No joined spaces to choose from",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(start = 24.dp),
                        )
                    }
                }
                if (info.supportsKnock) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            "Allow asking to join",
                            style = MaterialTheme.typography.bodyLarge,
                            modifier = Modifier.weight(1f),
                        )
                        Switch(
                            checked = knock && knockApplies,
                            onCheckedChange = { knock = it },
                            enabled = info.canChange && knockApplies,
                        )
                    }
                }
                if (!info.canChange) {
                    Text(
                        "You do not have permission to change this",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                error?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = info.canChange && !busy &&
                    (selection != FfiJoinRuleValue.RESTRICTED || spaceId != null),
                onClick = {
                    busy = true
                    error = null
                    val value = when (selection) {
                        FfiJoinRuleValue.PUBLIC -> FfiJoinRuleValue.PUBLIC
                        FfiJoinRuleValue.RESTRICTED ->
                            if (knock && info.supportsKnockRestricted) {
                                FfiJoinRuleValue.KNOCK_RESTRICTED
                            } else {
                                FfiJoinRuleValue.RESTRICTED
                            }
                        else ->
                            if (knock && info.supportsKnock) {
                                FfiJoinRuleValue.KNOCK
                            } else {
                                FfiJoinRuleValue.INVITE
                            }
                    }
                    val space =
                        if (selection == FfiJoinRuleValue.RESTRICTED) spaceId else null
                    state.setJoinRule(value, space) { failure ->
                        busy = false
                        if (failure == null) onDismiss() else error = failure
                    }
                },
            ) { Text("Save") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

/// Who can read the room's history.
@Composable
internal fun HistoryVisibilityDialog(state: CommuneState, onDismiss: () -> Unit) {
    val info = state.historyVisibilityInfo
    if (info == null) {
        AlertDialog(
            onDismissRequest = onDismiss,
            title = { Text("History Visibility") },
            text = { Text("Loading…") },
            confirmButton = { TextButton(onClick = onDismiss) { Text("Close") } },
        )
        return
    }

    var selection by remember { mutableStateOf(info.value) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }

    val options = listOf(
        FfiHistoryVisibility.SHARED to "Members, from when they could know of the room",
        FfiHistoryVisibility.INVITED to "Members, from their invitation",
        FfiHistoryVisibility.JOINED to "Members, from when they joined",
        FfiHistoryVisibility.WORLD_READABLE to "Anyone, even without joining",
    )

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("History Visibility") },
        text = {
            Column {
                for ((value, label) in options) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable(enabled = info.canChange) { selection = value },
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(
                            selected = selection == value,
                            onClick = { selection = value },
                            enabled = info.canChange,
                        )
                        Text(label, style = MaterialTheme.typography.bodyMedium)
                    }
                }
                if (!info.canChange) {
                    Text(
                        "You do not have permission to change this",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                error?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = info.canChange && !busy,
                onClick = {
                    busy = true
                    error = null
                    state.setHistoryVisibility(selection) { failure ->
                        busy = false
                        if (failure == null) onDismiss() else error = failure
                    }
                },
            ) { Text("Save") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

/// Upgrade the room to a newer version.
@Composable
internal fun UpgradeRoomDialog(state: CommuneState, onDismiss: () -> Unit) {
    val info = state.upgradeInfo
    var selected by remember(info) {
        mutableStateOf(
            info?.let { (it.stable + it.unstable).getOrNull(it.selectedIndex.toInt()) }
        )
    }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Upgrade Room") },
        text = {
            Column {
                if (info == null) {
                    Text("Loading…")
                } else {
                    Text(
                        "Upgrading a room closes it and creates a fresh one at the " +
                            "new version; members are invited to move over. " +
                            "Current version: ${info.currentVersion}",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    Spacer(Modifier.size(8.dp))
                    for (version in info.stable) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable { selected = version },
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(
                                selected = selected == version,
                                onClick = { selected = version },
                            )
                            Text("Version $version", style = MaterialTheme.typography.bodyLarge)
                        }
                    }
                    for (version in info.unstable) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable { selected = version },
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(
                                selected = selected == version,
                                onClick = { selected = version },
                            )
                            Text(
                                "Version $version (experimental)",
                                style = MaterialTheme.typography.bodyLarge,
                            )
                        }
                    }
                    if (!info.canUpgrade) {
                        Text(
                            "You do not have permission to upgrade this room",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                error?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = info?.canUpgrade == true && selected != null && !busy,
                onClick = {
                    busy = true
                    error = null
                    state.upgradeRoom(selected.orEmpty()) { failure ->
                        busy = false
                        if (failure == null) onDismiss() else error = failure
                    }
                },
            ) { Text("Upgrade", color = MaterialTheme.colorScheme.error) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

/// The addresses subpage: canonical and alternative public addresses,
/// this server's registered addresses, and the directory switch.
@Composable
fun AddressesScreen(state: CommuneState) {
    val addresses = state.addresses

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState()),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = { state.closeAddresses() }) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
            Text("Addresses", style = MaterialTheme.typography.titleMedium)
        }

        if (addresses == null) {
            LoadingFace(modifier = Modifier.fillMaxSize())
            return@Column
        }

        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text("Publish in Directory", style = MaterialTheme.typography.bodyLarge)
                Text(
                    "List this room in the server's public directory",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Switch(
                checked = addresses.published,
                onCheckedChange = { state.addressAction(FfiAddressAction.Publish(it)) },
            )
        }
        HorizontalDivider()

        Text(
            "Public addresses",
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        )
        addresses.canonical?.let { canonical ->
            AddressRow(
                address = canonical,
                tag = "Main",
                canChange = addresses.canChange,
                onRemove = {
                    state.addressAction(FfiAddressAction.RemoveCanonical(canonical))
                },
            )
        }
        for (alias in addresses.alt) {
            AddressRow(
                address = alias,
                tag = null,
                canChange = addresses.canChange,
                onRemove = { state.addressAction(FfiAddressAction.RemoveAlt(alias)) },
                onMakeMain = {
                    state.addressAction(FfiAddressAction.SetCanonical(alias))
                },
            )
        }
        if (addresses.canChange) {
            var newPublic by remember { mutableStateOf("") }
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OutlinedTextField(
                    value = newPublic,
                    onValueChange = { newPublic = it },
                    placeholder = { Text("#room:server") },
                    singleLine = true,
                    modifier = Modifier.weight(1f),
                )
                TextButton(
                    enabled = newPublic.isNotBlank(),
                    onClick = {
                        val alias = newPublic.trim()
                        if (addresses.canonical == null) {
                            state.addressAction(FfiAddressAction.SetCanonical(alias))
                        } else {
                            state.addressAction(FfiAddressAction.AddAlt(alias))
                        }
                        newPublic = ""
                    },
                ) { Text("Add") }
            }
        }
        HorizontalDivider()

        Text(
            "Registered on this server",
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        )
        for (alias in addresses.local) {
            AddressRow(
                address = alias,
                tag = null,
                canChange = true,
                onRemove = {
                    state.addressAction(FfiAddressAction.UnregisterLocal(alias))
                },
            )
        }
        var newLocal by remember { mutableStateOf("") }
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OutlinedTextField(
                value = newLocal,
                onValueChange = { newLocal = it },
                placeholder = { Text("#room:server") },
                singleLine = true,
                modifier = Modifier.weight(1f),
            )
            TextButton(
                enabled = newLocal.isNotBlank(),
                onClick = {
                    state.addressAction(FfiAddressAction.RegisterLocal(newLocal.trim()))
                    newLocal = ""
                },
            ) { Text("Register") }
        }
    }
}

@Composable
private fun AddressRow(
    address: String,
    tag: String?,
    canChange: Boolean,
    onRemove: () -> Unit,
    onMakeMain: (() -> Unit)? = null,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            address,
            style = MaterialTheme.typography.bodyLarge,
            modifier = Modifier.weight(1f),
        )
        tag?.let {
            Text(
                it,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.primary,
            )
        }
        if (canChange) {
            onMakeMain?.let {
                TextButton(onClick = it) { Text("Make Main") }
            }
            IconButton(onClick = onRemove) {
                Icon(
                    Icons.Filled.Close,
                    contentDescription = "Remove $address",
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

/// The server ACL subpage: which servers' users may take part.
@Composable
fun ServerAclScreen(state: CommuneState) {
    val acl = state.serverAcl

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState()),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = { state.closeServerAcl() }) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
            Text("Server ACL", style = MaterialTheme.typography.titleMedium)
        }

        if (acl == null) {
            LoadingFace(modifier = Modifier.fillMaxSize())
            return@Column
        }

        var allowed by remember { mutableStateOf(acl.allow) }
        var denied by remember { mutableStateOf(acl.deny) }
        var ipLiterals by remember { mutableStateOf(acl.allowIpLiterals) }
        var error by remember { mutableStateOf<String?>(null) }
        var busy by remember { mutableStateOf(false) }

        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text("Allow IP Literals", style = MaterialTheme.typography.bodyLarge)
                Text(
                    "Servers named by a raw IP address",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Switch(
                checked = ipLiterals,
                onCheckedChange = { ipLiterals = it },
                enabled = acl.canChange,
            )
        }

        @Composable
        fun serverList(
            title: String,
            servers: List<String>,
            onChange: (List<String>) -> Unit,
        ) {
            Text(
                title,
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            )
            for (server in servers) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        server,
                        style = MaterialTheme.typography.bodyLarge,
                        modifier = Modifier.weight(1f),
                    )
                    if (acl.canChange) {
                        IconButton(onClick = { onChange(servers - server) }) {
                            Icon(
                                Icons.Filled.Close,
                                contentDescription = "Remove $server",
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
            if (acl.canChange) {
                var newServer by remember { mutableStateOf("") }
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    OutlinedTextField(
                        value = newServer,
                        onValueChange = { newServer = it },
                        placeholder = { Text("example.org or *.example.org") },
                        singleLine = true,
                        modifier = Modifier.weight(1f),
                    )
                    TextButton(
                        enabled = newServer.isNotBlank(),
                        onClick = {
                            onChange(servers + newServer.trim())
                            newServer = ""
                        },
                    ) { Text("Add") }
                }
            }
        }

        serverList("Allowed servers", allowed) { allowed = it }
        serverList("Denied servers", denied) { denied = it }

        error?.let {
            Text(
                it,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
        }
        if (acl.canChange) {
            TextButton(
                enabled = !busy,
                onClick = {
                    busy = true
                    error = null
                    state.saveServerAcl(allowed, denied, ipLiterals) { failure ->
                        busy = false
                        if (failure == null) state.closeServerAcl() else error = failure
                    }
                },
                modifier = Modifier.padding(horizontal = 16.dp),
            ) { Text("Save") }
        }
    }
}

/// The permissions subpage: every threshold of the power-levels matrix.
@Composable
fun PermissionsScreen(state: CommuneState) {
    val matrix = state.permissionsMatrix

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState()),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = { state.closePermissions() }) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
            Text("Permissions", style = MaterialTheme.typography.titleMedium)
        }

        if (matrix == null) {
            LoadingFace(modifier = Modifier.fillMaxSize())
            return@Column
        }

        var edited by remember { mutableStateOf(matrix) }
        var error by remember { mutableStateOf<String?>(null) }
        var busy by remember { mutableStateOf(false) }

        @Composable
        fun levelRow(title: String, value: Long, onChange: (Long) -> Unit) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 2.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    title,
                    style = MaterialTheme.typography.bodyLarge,
                    modifier = Modifier.weight(1f),
                )
                OutlinedTextField(
                    value = value.toString(),
                    onValueChange = { text ->
                        text.toLongOrNull()?.let(onChange)
                    },
                    enabled = matrix.canChange,
                    singleLine = true,
                    modifier = Modifier.width(88.dp),
                )
            }
        }

        @Composable
        fun section(title: String) {
            Text(
                title,
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            )
        }

        section("Roles")
        levelRow("Default member level", edited.usersDefault) {
            edited = edited.copy(usersDefault = it)
        }

        section("Actions")
        levelRow("Send messages", edited.eventsDefault) {
            edited = edited.copy(eventsDefault = it)
        }
        levelRow("Remove own messages", edited.redactOwn) {
            edited = edited.copy(redactOwn = it)
        }
        levelRow("Remove others' messages", edited.redactOthers) {
            edited = edited.copy(redactOthers = it)
        }
        levelRow("Notify the whole room", edited.notifyRoom) {
            edited = edited.copy(notifyRoom = it)
        }
        levelRow("Invite", edited.invite) { edited = edited.copy(invite = it) }
        levelRow("Kick", edited.kick) { edited = edited.copy(kick = it) }
        levelRow("Ban", edited.ban) { edited = edited.copy(ban = it) }

        section("Room changes")
        levelRow("Change any state", edited.stateDefault) {
            edited = edited.copy(stateDefault = it)
        }
        levelRow("Change the name", edited.name) { edited = edited.copy(name = it) }
        levelRow("Change the topic", edited.topic) { edited = edited.copy(topic = it) }
        levelRow("Change the avatar", edited.avatar) { edited = edited.copy(avatar = it) }
        levelRow("Change the addresses", edited.aliases) {
            edited = edited.copy(aliases = it)
        }
        levelRow("Change the history visibility", edited.historyVisibility) {
            edited = edited.copy(historyVisibility = it)
        }
        levelRow("Enable encryption", edited.encryption) {
            edited = edited.copy(encryption = it)
        }
        levelRow("Change these permissions", edited.powerLevels) {
            edited = edited.copy(powerLevels = it)
        }
        levelRow("Change the server ACL", edited.serverAcl) {
            edited = edited.copy(serverAcl = it)
        }
        levelRow("Upgrade the room", edited.upgrade) { edited = edited.copy(upgrade = it) }

        if (!matrix.canChange) {
            Text(
                "You do not have permission to change these",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
        }
        error?.let {
            Text(
                it,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
        }
        if (matrix.canChange) {
            TextButton(
                enabled = !busy,
                onClick = {
                    busy = true
                    error = null
                    state.savePermissions(edited) { failure ->
                        busy = false
                        if (failure == null) state.closePermissions() else error = failure
                    }
                },
                modifier = Modifier.padding(horizontal = 16.dp),
            ) { Text("Save") }
        }
    }
}
