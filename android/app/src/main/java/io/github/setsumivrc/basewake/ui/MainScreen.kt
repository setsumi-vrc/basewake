@file:OptIn(ExperimentalMaterial3Api::class)

package io.github.setsumivrc.basewake.ui

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Snackbar
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.setsumivrc.basewake.MainActivity
import io.github.setsumivrc.basewake.data.OffMode
import io.github.setsumivrc.basewake.data.Station
import kotlinx.coroutines.launch

private val SidePadding = 20.dp

@Composable
fun MainScreen(vm: MainViewModel) {
    val state by vm.state.collectAsStateWithLifecycle()
    val activity = LocalContext.current as MainActivity
    val snackbar = remember { SnackbarHostState() }
    var renaming by remember { mutableStateOf<Station?>(null) }

    LaunchedEffect(vm) {
        vm.messages.collect { message ->
            launch {
                snackbar.currentSnackbarData?.dismiss()
                snackbar.showSnackbar(message)
            }
        }
    }

    // Permissions: ask once on first launch, then from the banner.
    var asked by rememberSaveable { mutableStateOf(false) }
    var permanentlyDenied by rememberSaveable { mutableStateOf(false) }
    val permissionLauncher = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        val granted = activity.hasPermissions()
        permanentlyDenied = !granted &&
            MainActivity.PERMISSIONS.none { activity.shouldShowRequestPermissionRationale(it) }
        vm.onResume(granted)
    }
    LaunchedEffect(Unit) {
        if (!asked && !activity.hasPermissions()) {
            asked = true
            permissionLauncher.launch(MainActivity.PERMISSIONS)
        }
    }
    val enableLauncher = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) {
        vm.onResume(activity.hasPermissions())
    }

    val problem: Pair<String, Pair<String, () -> Unit>?>? = when (state.readiness) {
        Readiness.Ready -> null
        Readiness.NoBluetooth -> "This phone has no Bluetooth LE." to null
        Readiness.NoPermission -> {
            val text = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                "Basewake needs the Nearby devices permission to find and switch base stations."
            } else {
                "Basewake needs Location access to scan for Bluetooth devices on this Android version. " +
                    "It never uses your location."
            }
            val action = if (permanentlyDenied) {
                "Open settings" to {
                    activity.startActivity(
                        Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", activity.packageName, null)),
                    )
                }
            } else {
                "Allow" to { permissionLauncher.launch(MainActivity.PERMISSIONS) }
            }
            text to action
        }
        Readiness.BluetoothOff -> "Bluetooth is off." to ("Turn on" to { requestBluetooth(enableLauncher::launch) })
    }

    Scaffold(
        containerColor = Background,
        contentWindowInsets = WindowInsets.safeDrawing,
        snackbarHost = {
            SnackbarHost(snackbar) {
                Snackbar(it, containerColor = Raised, contentColor = TextPrimary, shape = RoundedCornerShape(8.dp))
            }
        },
    ) { insets ->
        val dir = LocalLayoutDirection.current
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(
                start = insets.calculateStartPadding(dir) + SidePadding,
                end = insets.calculateEndPadding(dir) + SidePadding,
                top = insets.calculateTopPadding() + 24.dp,
                bottom = insets.calculateBottomPadding() + 24.dp,
            ),
        ) {
            item(key = "header") { Header(state, vm::setAll) }
            problem?.let { (text, action) ->
                item(key = "problem") { Problem(text, action) }
            }
            item(key = "gap") { Spacer(Modifier.padding(top = 18.dp)) }

            if (state.stations.isEmpty()) {
                item(key = "empty") { Empty(state.scanning, vm::scan) }
            } else {
                item(key = "top-divider") { Divider() }
                items(state.stations, key = { it.station.mac }) { s ->
                    StationRow(
                        s = s,
                        ready = state.readiness == Readiness.Ready,
                        onSwitch = { on -> vm.setPower(s.station.mac, on) },
                        onRename = { renaming = s.station },
                        onManaged = { vm.setManaged(s.station.mac, it) },
                        onRemove = { vm.remove(s.station.mac) },
                    )
                    Divider()
                }
                item(key = "scan") { ScanRow(state.scanning, vm::scan) }
            }

            item(key = "settings") { SettingsSection(state.offMode, vm::setOffMode) }
        }
    }

    renaming?.let { station ->
        RenameDialog(
            station = station,
            onDismiss = { renaming = null },
            onSave = {
                vm.rename(station.mac, it)
                renaming = null
            },
        )
    }
}

@SuppressLint("MissingPermission")
private fun requestBluetooth(launch: (Intent) -> Unit) {
    try {
        launch(Intent(BluetoothAdapter.ACTION_REQUEST_ENABLE))
    } catch (e: ActivityNotFoundException) {
        // No system dialog to turn Bluetooth on; the banner stays.
    } catch (e: SecurityException) {
        // Permission was revoked meanwhile; the banner will ask for it.
    }
}

@Composable
private fun Header(state: UiState, onMaster: (Boolean) -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text("Base stations", color = TextPrimary, fontSize = 24.sp, fontWeight = FontWeight.SemiBold)
            Spacer(Modifier.padding(top = 2.dp))
            Text(
                when {
                    state.scanning -> "Scanning…"
                    state.total == 0 -> "None managed"
                    else -> "${state.on} of ${state.total} on"
                },
                color = TextSecondary,
                fontSize = 14.sp,
            )
        }
        Switch(
            checked = state.masterOn,
            onCheckedChange = onMaster,
            enabled = state.total > 0,
            colors = switchColors(),
        )
        // Line up with the station switches, which sit left of each row's menu button.
        Spacer(Modifier.width(48.dp))
    }
}

@Composable
private fun Problem(text: String, action: Pair<String, () -> Unit>?) {
    Row(Modifier.fillMaxWidth().padding(top = 14.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(text, color = Danger, fontSize = 14.sp, modifier = Modifier.weight(1f))
        if (action != null) {
            TextButton(onClick = action.second) { Text(action.first, color = TextPrimary) }
        }
    }
}

@Composable
private fun Divider() {
    HorizontalDivider(thickness = Dp.Hairline, color = Hairline)
}

@Composable
private fun StationRow(
    s: StationUi,
    ready: Boolean,
    onSwitch: (Boolean) -> Unit,
    onRename: () -> Unit,
    onManaged: (Boolean) -> Unit,
    onRemove: () -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    val (label, labelColor) = when {
        s.pending == true -> "Turning on…" to TextSecondary
        s.pending == false -> "Turning off…" to TextSecondary
        s.power == null -> (if (ready) "Checking…" else "") to TextTertiary
        s.power == PowerState.Unreachable -> PowerState.Unreachable.label to Danger
        else -> (s.power?.label ?: "") to TextSecondary
    }
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                s.station.name,
                color = if (s.station.managed) TextPrimary else TextSecondary,
                fontSize = 16.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Spacer(Modifier.padding(top = 2.dp))
            Text(
                buildAnnotatedString {
                    withStyle(SpanStyle(fontFamily = FontFamily.Monospace, color = TextTertiary)) {
                        append(s.station.mac)
                    }
                    if (label.isNotEmpty()) {
                        withStyle(SpanStyle(color = TextTertiary)) { append("  ·  ") }
                        withStyle(SpanStyle(color = labelColor)) { append(label) }
                    }
                    if (!s.station.managed) {
                        withStyle(SpanStyle(color = TextTertiary)) { append("  ·  Not managed") }
                    }
                },
                fontSize = 13.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Switch(checked = s.switchOn, onCheckedChange = onSwitch, colors = switchColors())
        Box {
            IconButton(onClick = { menu = true }) {
                Icon(Icons.Filled.MoreVert, contentDescription = "More for ${s.station.name}", tint = TextTertiary)
            }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                DropdownMenuItem(text = { Text("Rename") }, onClick = {
                    menu = false
                    onRename()
                })
                DropdownMenuItem(
                    text = { Text(if (s.station.managed) "Stop managing" else "Manage") },
                    onClick = {
                        menu = false
                        onManaged(!s.station.managed)
                    },
                )
                DropdownMenuItem(text = { Text("Remove", color = Danger) }, onClick = {
                    menu = false
                    onRemove()
                })
            }
        }
    }
}

@Composable
private fun ScanRow(scanning: Boolean, onScan: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(enabled = !scanning, onClick = onScan)
            .padding(vertical = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text(
            if (scanning) "Scanning…" else "Scan for base stations",
            color = if (scanning) TextSecondary else TextPrimary,
            fontSize = 15.sp,
        )
        if (scanning) {
            CircularProgressIndicator(Modifier.size(14.dp), color = TextSecondary, strokeWidth = 2.dp)
        }
    }
}

@Composable
private fun Empty(scanning: Boolean, onScan: () -> Unit) {
    Column {
        Divider()
        Spacer(Modifier.padding(top = 20.dp))
        Text("No base stations yet", color = TextPrimary, fontSize = 16.sp)
        Spacer(Modifier.padding(top = 4.dp))
        Text("Plug them in, then scan to find them.", color = TextSecondary, fontSize = 14.sp)
        Spacer(Modifier.padding(top = 8.dp))
        ScanRow(scanning, onScan)
        Divider()
    }
}

@Composable
private fun SettingsSection(offMode: OffMode, onOffMode: (OffMode) -> Unit) {
    Column(Modifier.padding(top = 30.dp)) {
        Text("Settings", color = TextSecondary, fontSize = 13.sp, fontWeight = FontWeight.Medium)
        Spacer(Modifier.padding(top = 8.dp))
        Divider()
        Row(
            Modifier.fillMaxWidth().padding(vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f).padding(end = 12.dp)) {
                Text("Turn off as", color = TextPrimary, fontSize = 16.sp)
                Spacer(Modifier.padding(top = 2.dp))
                Text("Sleep is fully off. Standby wakes faster.", color = TextSecondary, fontSize = 13.sp)
            }
            SingleChoiceSegmentedButtonRow {
                OffMode.entries.forEachIndexed { i, mode ->
                    SegmentedButton(
                        selected = offMode == mode,
                        onClick = { onOffMode(mode) },
                        shape = SegmentedButtonDefaults.itemShape(i, OffMode.entries.size),
                        colors = SegmentedButtonDefaults.colors(
                            activeContainerColor = Hairline,
                            activeContentColor = TextPrimary,
                            activeBorderColor = Hairline,
                            inactiveContainerColor = Color.Transparent,
                            inactiveContentColor = TextSecondary,
                            inactiveBorderColor = Hairline,
                        ),
                        icon = {},
                    ) {
                        Text(mode.label, fontSize = 13.sp)
                    }
                }
            }
        }
        Divider()
    }
}

@Composable
private fun RenameDialog(station: Station, onDismiss: () -> Unit, onSave: (String) -> Unit) {
    var text by remember(station.mac) { mutableStateOf(station.name) }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = Raised,
        title = { Text("Rename") },
        text = {
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                singleLine = true,
                colors = OutlinedTextFieldDefaults.colors(
                    focusedBorderColor = TextSecondary,
                    unfocusedBorderColor = Hairline,
                    cursorColor = TextPrimary,
                    focusedTextColor = TextPrimary,
                    unfocusedTextColor = TextPrimary,
                ),
            )
        },
        confirmButton = {
            TextButton(onClick = { onSave(text) }, enabled = text.isNotBlank()) { Text("Save") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}
