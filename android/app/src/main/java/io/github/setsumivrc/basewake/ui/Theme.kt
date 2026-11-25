package io.github.setsumivrc.basewake.ui

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SwitchColors
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

// The desktop app's palette. Green only ever means "on".
val Background = Color(0xFF121316)
val Raised = Color(0xFF1B1D21)
val Hairline = Color(0xFF26292E)
val TextPrimary = Color(0xFFE7E9EC)
val TextSecondary = Color(0xFF959BA5)
val TextTertiary = Color(0xFF626871)
val Accent = Color(0xFF5CC98D)
val Danger = Color(0xFFEC6F72)

private val Colors = darkColorScheme(
    primary = TextPrimary,
    onPrimary = Background,
    primaryContainer = Raised,
    onPrimaryContainer = TextPrimary,
    secondary = TextSecondary,
    onSecondary = Background,
    secondaryContainer = Hairline,
    onSecondaryContainer = TextPrimary,
    background = Background,
    onBackground = TextPrimary,
    surface = Background,
    onSurface = TextPrimary,
    surfaceVariant = Raised,
    onSurfaceVariant = TextSecondary,
    surfaceContainerLowest = Background,
    surfaceContainerLow = Raised,
    surfaceContainer = Raised,
    surfaceContainerHigh = Raised,
    surfaceContainerHighest = Raised,
    surfaceTint = Color.Transparent,
    inverseSurface = Hairline,
    inverseOnSurface = TextPrimary,
    inversePrimary = TextPrimary,
    outline = Hairline,
    outlineVariant = Hairline,
    error = Danger,
    onError = Background,
)

@Composable
fun BasewakeTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = Colors, content = content)
}

@Composable
fun switchColors(): SwitchColors = SwitchDefaults.colors(
    checkedThumbColor = Background,
    checkedTrackColor = Accent,
    checkedBorderColor = Accent,
    uncheckedThumbColor = TextTertiary,
    uncheckedTrackColor = Raised,
    uncheckedBorderColor = Hairline,
)
