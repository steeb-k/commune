// The sticker/GIF picker: a bottom sheet with the account's sticker
// packs on one tab and the KLIPY GIF search on the other — the GTK
// sticker picker's two halves. Previews arrive as bytes or cache files
// from the core, decoded small; the full media shows in the room.
package io.github.steeb_k.commune.ui

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Tab
import androidx.compose.material3.TabRow
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.R
import io.github.steeb_k.commune.core.FfiGif
import io.github.steeb_k.commune.core.FfiSticker

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun GifPickerSheet(state: CommuneState) {
    var tab by remember { mutableIntStateOf(0) }
    LaunchedEffect(Unit) { state.loadStickerPacks() }

    ModalBottomSheet(onDismissRequest = { state.closeGifPicker() }) {
        TabRow(selectedTabIndex = tab) {
            Tab(selected = tab == 0, onClick = { tab = 0 }, text = { Text("Stickers") })
            Tab(selected = tab == 1, onClick = { tab = 1 }, text = { Text("GIFs") })
        }

        when (tab) {
            0 -> StickerTab(state)
            else -> GifTab(state)
        }
    }
}

@Composable
private fun StickerTab(state: CommuneState) {
    when {
        !state.stickerPacksLoaded ->
            LoadingRing(modifier = Modifier.fillMaxWidth().height(280.dp))

        state.stickerPacks.isEmpty() -> Text(
            "No sticker packs on this account yet. Packs follow the " +
                "image-pack convention (im.ponies), shared with the other " +
                "clients that use it.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(16.dp).height(240.dp),
        )

        else -> LazyVerticalGrid(
            columns = GridCells.Fixed(4),
            modifier = Modifier.fillMaxWidth().height(420.dp).padding(horizontal = 12.dp),
        ) {
            for (pack in state.stickerPacks) {
                item(key = "pack:${pack.name}", span = { GridItemSpan(maxLineSpan) }) {
                    Text(
                        pack.name,
                        style = MaterialTheme.typography.titleSmall,
                        modifier = Modifier.padding(vertical = 8.dp),
                    )
                }
                items(pack.stickers.size, key = { "${pack.name}:${pack.stickers[it].url}" }) {
                    StickerCell(state, pack.stickers[it])
                }
            }
        }
    }
}

@Composable
private fun StickerCell(state: CommuneState, sticker: FfiSticker) {
    val path = state.stickerMedia[sticker.url]

    Box(
        modifier = Modifier
            .padding(4.dp)
            .fillMaxWidth()
            .aspectRatio(1f)
            .clip(RoundedCornerShape(8.dp))
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .clickable { state.sendSticker(sticker) },
        contentAlignment = Alignment.Center,
    ) {
        if (path != null) {
            MediaImage(
                path,
                contentDescription = sticker.body,
                modifier = Modifier.fillMaxWidth(),
                targetSizePx = 216,
            )
        }
    }
}

@Composable
private fun GifTab(state: CommuneState) {
    var query by remember { mutableStateOf("") }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        OutlinedTextField(
            value = query,
            onValueChange = { query = it },
            placeholder = { Text("Search GIFs") },
            singleLine = true,
            modifier = Modifier.weight(1f),
        )
        IconButton(
            enabled = query.isNotBlank() && !state.gifBusy,
            onClick = { state.searchGifs(query.trim()) },
        ) {
            Icon(
                painterResource(R.drawable.ic_system_search_symbolic),
                contentDescription = "Search",
            )
        }
    }

    when {
        state.gifResults.isEmpty() && state.gifBusy ->
            LoadingRing(modifier = Modifier.fillMaxWidth().height(280.dp))

        state.gifResults.isEmpty() -> Text(
            "Search for a GIF to send.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(16.dp).height(240.dp),
        )

        else -> LazyVerticalGrid(
            columns = GridCells.Fixed(2),
            modifier = Modifier.fillMaxWidth().height(420.dp).padding(horizontal = 12.dp),
        ) {
            items(state.gifResults.size, key = { state.gifResults[it].id }) { index ->
                GifCell(state, state.gifResults[index])
                // The last row coming into view asks for the next page.
                if (index == state.gifResults.size - 1 && state.gifHasNext) {
                    LaunchedEffect(index) { state.loadMoreGifs() }
                }
            }
        }
    }
}

@Composable
private fun GifCell(state: CommuneState, gif: FfiGif) {
    val bytes = state.gifPreviews[gif.previewUrl]
    val bitmap = remember(bytes) {
        bytes?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }
    }
    val ratio = if (gif.previewHeight > 0u) {
        gif.previewWidth.toFloat() / gif.previewHeight.toFloat()
    } else {
        1f
    }

    Box(
        modifier = Modifier
            .padding(4.dp)
            .fillMaxWidth()
            .aspectRatio(ratio.coerceIn(0.5f, 2f))
            .clip(RoundedCornerShape(8.dp))
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .clickable { state.sendGif(gif) },
        contentAlignment = Alignment.Center,
    ) {
        if (bitmap != null) {
            Image(
                bitmap.asImageBitmap(),
                contentDescription = gif.title,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}
