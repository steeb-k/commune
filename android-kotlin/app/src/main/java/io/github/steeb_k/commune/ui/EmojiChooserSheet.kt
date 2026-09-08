// The emoji chooser: GTK's GtkEmojiChooser, which the message context
// menu's "More Reactions" button opens in place of the quick reactions.
// The same sections in the same order, fed by the same emojibase dataset
// (assets/emoji.json, see gen-emoji-data.py), the same search — every word
// typed a prefix of a word of the name or of a keyword — and a long press
// on an emoji with skin tones opening its six variants. What the chooser
// picked recently is its own list, on this device, like GTK's setting.
package io.github.steeb_k.commune.ui

import android.content.Context
import android.graphics.Paint
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.outlined.EmojiSymbols
import androidx.compose.material.icons.outlined.Flag
import androidx.compose.material.icons.outlined.Flight
import androidx.compose.material.icons.outlined.History
import androidx.compose.material.icons.outlined.Lightbulb
import androidx.compose.material.icons.outlined.PanTool
import androidx.compose.material.icons.outlined.Pets
import androidx.compose.material.icons.outlined.Restaurant
import androidx.compose.material.icons.outlined.SentimentSatisfied
import androidx.compose.material.icons.outlined.SportsSoccer
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import java.text.Normalizer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray

/// One emoji of the dataset: what the GTK chooser keeps of each.
class Emoji(
    val emoji: String,
    val label: String,
    /// The emojibase group, the section key.
    val group: Int,
    /// The five skin-tone variants, light to dark; empty without tones.
    val variants: List<String>,
    /// The folded words of the label and the keywords, what a search
    /// term has to be a prefix of.
    val tokens: List<String>,
)

/// The dataset, read from the assets once.
object EmojiData {
    @Volatile
    private var loaded: List<Emoji>? = null

    fun load(context: Context): List<Emoji> {
        loaded?.let { return it }
        synchronized(this) {
            loaded?.let { return it }
            val text = context.assets.open("emoji.json").bufferedReader().use { it.readText() }
            val array = JSONArray(text)
            val list = ArrayList<Emoji>(array.length())
            // The dataset runs ahead of the device's font: an emoji it
            // cannot draw would show as a box, so it is left out, as the
            // GTK data, frozen at its release, never lists one.
            val paint = Paint()
            for (i in 0 until array.length()) {
                val entry = array.getJSONArray(i)
                val emoji = entry.getString(0)
                if (!paint.hasGlyph(emoji)) continue
                val label = entry.getString(1)
                val tags = entry.getJSONArray(2)
                val tokens = ArrayList<String>(tokenize(label))
                for (t in 0 until tags.length()) tokens.addAll(tokenize(tags.getString(t)))
                val skins = entry.getJSONArray(4)
                val variants = (0 until skins.length())
                    .map { skins.getString(it) }
                    .filter { paint.hasGlyph(it) }
                list.add(Emoji(emoji, label, entry.getInt(3), variants, tokens))
            }
            loaded = list
            return list
        }
    }

    /// Split into words, case folded and stripped of accents, as GTK's
    /// g_str_tokenize_and_fold does for both the search and the names.
    fun tokenize(text: String): List<String> {
        val folded = Normalizer.normalize(text, Normalizer.Form.NFKD)
            .filter { Character.getType(it) != Character.NON_SPACING_MARK.toInt() }
            .lowercase()
        return folded.split(Regex("[^\\p{L}\\p{N}]+")).filter { it.isNotEmpty() }
    }

    /// GTK's match_tokens: every term is a prefix of some word.
    fun matches(terms: List<String>, emoji: Emoji): Boolean =
        terms.all { term -> emoji.tokens.any { it.startsWith(term) } }
}

/// The emoji picked lately, most recent first — GTK's recently-used-emoji
/// setting, sized like it and kept on this device like it.
object EmojiRecents {
    private const val MAX_RECENT = 7 * 3

    fun load(context: Context): List<String> {
        val stored = context.getSharedPreferences("emoji-chooser", Context.MODE_PRIVATE)
            .getString("recently-used-emoji", null) ?: return emptyList()
        return try {
            val array = JSONArray(stored)
            (0 until array.length()).map { array.getString(it) }
        } catch (_: Exception) {
            emptyList()
        }
    }

    fun add(context: Context, emoji: String): List<String> {
        val list = ArrayList<String>()
        list.add(emoji)
        for (other in load(context)) {
            if (other == emoji) continue
            if (list.size >= MAX_RECENT) break
            list.add(other)
        }
        context.getSharedPreferences("emoji-chooser", Context.MODE_PRIVATE)
            .edit()
            .putString("recently-used-emoji", JSONArray(list).toString())
            .apply()
        return list
    }
}

/// A section of the chooser: GTK's, in GTK's order, keyed on the
/// emojibase group that fills it.
private class Section(val group: Int, val title: String, val icon: ImageVector)

private const val RECENT_GROUP = -1

private val SECTIONS = listOf(
    Section(RECENT_GROUP, "Recent", Icons.Outlined.History),
    Section(0, "Smileys & People", Icons.Outlined.SentimentSatisfied),
    Section(1, "Body & Clothing", Icons.Outlined.PanTool),
    Section(3, "Animals & Nature", Icons.Outlined.Pets),
    Section(4, "Food & Drink", Icons.Outlined.Restaurant),
    Section(5, "Travel & Places", Icons.Outlined.Flight),
    Section(6, "Activities", Icons.Outlined.SportsSoccer),
    Section(7, "Objects", Icons.Outlined.Lightbulb),
    Section(8, "Symbols", Icons.Outlined.EmojiSymbols),
    Section(9, "Flags", Icons.Outlined.Flag),
)

/// What the grid shows, in order: a heading spanning the row, or a cell.
private sealed class Entry(val section: Section) {
    class Heading(section: Section) : Entry(section)
    class Cell(section: Section, val text: String, val data: Emoji) : Entry(section)
}

/// The chooser as a bottom sheet. Picking an emoji calls back with its
/// text, skin tone included, and closes the sheet.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun EmojiChooserSheet(onPick: (String) -> Unit, onDismiss: () -> Unit) {
    val context = LocalContext.current
    var all by remember { mutableStateOf<List<Emoji>?>(null) }
    var recent by remember { mutableStateOf(EmojiRecents.load(context)) }
    var query by remember { mutableStateOf("") }
    val gridState = rememberLazyGridState()
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) {
        all = withContext(Dispatchers.IO) { EmojiData.load(context) }
    }

    val terms = remember(query) { EmojiData.tokenize(query) }
    val entries = remember(all, recent, terms) {
        val data = all ?: return@remember emptyList()
        val byText = HashMap<String, Emoji>(data.size * 2)
        for (emoji in data) {
            byText[emoji.emoji] = emoji
            for (variant in emoji.variants) byText[variant] = emoji
        }
        val entries = ArrayList<Entry>(data.size + SECTIONS.size)
        for (section in SECTIONS) {
            val cells = if (section.group == RECENT_GROUP) {
                recent.mapNotNull { text -> byText[text]?.let { Entry.Cell(section, text, it) } }
            } else {
                data.filter { it.group == section.group }.map { Entry.Cell(section, it.emoji, it) }
            }.filter { terms.isEmpty() || EmojiData.matches(terms, it.data) }
            // A section with nothing to show hides, heading and all: the
            // recent one until something was picked, the others under a
            // search that leaves them empty.
            if (cells.isEmpty()) continue
            entries.add(Entry.Heading(section))
            entries.addAll(cells)
        }
        entries
    }
    val headingIndex = remember(entries) {
        entries.withIndex()
            .filter { it.value is Entry.Heading }
            .associate { it.value.section.group to it.index }
    }
    val currentGroup by remember(entries) {
        derivedStateOf {
            entries.getOrNull(gridState.firstVisibleItemIndex)?.section?.group
        }
    }

    fun pick(cell: Entry.Cell) {
        if (cell.section.group != RECENT_GROUP) {
            recent = EmojiRecents.add(context, cell.text)
        }
        onPick(cell.text)
        onDismiss()
    }

    // Fully open at once, as the GTK popover is: half a sheet would leave
    // the section bar below the screen edge until a drag.
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheetState) {
        Column(
            modifier = Modifier
                .fillMaxHeight(0.85f)
                .imePadding(),
        ) {
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                placeholder = { Text("Search") },
                singleLine = true,
                leadingIcon = { Icon(Icons.Filled.Search, contentDescription = null) },
                trailingIcon = {
                    if (query.isNotEmpty()) {
                        IconButton(onClick = { query = "" }) {
                            Icon(Icons.Filled.Close, contentDescription = "Clear search")
                        }
                    }
                },
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                // Enter picks the first hit, as in the GTK search entry.
                keyboardActions = KeyboardActions(onSearch = {
                    entries.firstOrNull { it is Entry.Cell }?.let { pick(it as Entry.Cell) }
                }),
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 4.dp),
            )

            if (all != null && entries.isEmpty()) {
                Column(
                    modifier = Modifier.weight(1f).fillMaxWidth(),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center,
                ) {
                    Text("No Results Found", style = MaterialTheme.typography.titleLarge)
                    Spacer(Modifier.height(8.dp))
                    Text(
                        "Try a different search",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            } else {
                LazyVerticalGrid(
                    columns = GridCells.Adaptive(minSize = 44.dp),
                    state = gridState,
                    modifier = Modifier
                        .weight(1f)
                        .fillMaxWidth()
                        .padding(horizontal = 12.dp),
                ) {
                    items(
                        count = entries.size,
                        key = { index ->
                            when (val entry = entries[index]) {
                                is Entry.Heading -> "heading:${entry.section.group}"
                                is Entry.Cell -> "${entry.section.group}:${entry.text}"
                            }
                        },
                        span = { index ->
                            if (entries[index] is Entry.Heading) GridItemSpan(maxLineSpan) else GridItemSpan(1)
                        },
                        contentType = { index -> if (entries[index] is Entry.Heading) 0 else 1 },
                    ) { index ->
                        when (val entry = entries[index]) {
                            is Entry.Heading -> Text(
                                entry.section.title,
                                style = MaterialTheme.typography.titleSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(start = 4.dp, top = 12.dp, bottom = 4.dp),
                            )
                            is Entry.Cell -> EmojiCell(entry, onPick = ::pick)
                        }
                    }
                }
            }

            HorizontalDivider()
            // The section bar: one button a section, the one on screen
            // marked, a tap scrolling its heading to the top.
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 4.dp),
                horizontalArrangement = Arrangement.SpaceEvenly,
            ) {
                for (section in SECTIONS) {
                    val target = headingIndex[section.group]
                    IconButton(
                        onClick = {
                            target?.let { scope.launch { gridState.animateScrollToItem(it) } }
                        },
                        enabled = target != null,
                        modifier = Modifier.size(36.dp),
                    ) {
                        Icon(
                            section.icon,
                            contentDescription = section.title,
                            tint = if (currentGroup == section.group) {
                                MaterialTheme.colorScheme.primary
                            } else if (target != null) {
                                MaterialTheme.colorScheme.onSurfaceVariant
                            } else {
                                MaterialTheme.colorScheme.outlineVariant
                            },
                        )
                    }
                }
            }
            Spacer(Modifier.height(8.dp))
        }
    }
}

/// One emoji of the grid. A long press on one with skin tones opens its
/// variants, the plain one first, as the GTK chooser's popover does.
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun EmojiCell(cell: Entry.Cell, onPick: (Entry.Cell) -> Unit) {
    var variantsOpen by remember { mutableStateOf(false) }
    val hasVariants = cell.data.variants.isNotEmpty()

    Box(contentAlignment = Alignment.Center) {
        Text(
            cell.text,
            style = MaterialTheme.typography.headlineSmall,
            modifier = Modifier
                .clip(CircleShape)
                .combinedClickable(
                    onClick = { onPick(cell) },
                    onLongClick = if (hasVariants) ({ variantsOpen = true }) else null,
                )
                .padding(6.dp),
        )
        if (hasVariants) {
            DropdownMenu(expanded = variantsOpen, onDismissRequest = { variantsOpen = false }) {
                Row(modifier = Modifier.padding(horizontal = 4.dp)) {
                    for (variant in listOf(cell.data.emoji) + cell.data.variants) {
                        Text(
                            variant,
                            style = MaterialTheme.typography.headlineSmall,
                            modifier = Modifier
                                .clip(CircleShape)
                                .combinedClickable(onClick = {
                                    variantsOpen = false
                                    onPick(Entry.Cell(cell.section, variant, cell.data))
                                })
                                .padding(6.dp),
                        )
                    }
                }
            }
        }
    }
}
