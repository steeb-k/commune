package io.github.steeb_k.commune.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.foundation.text.appendInlineContent
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.Placeholder
import androidx.compose.ui.text.PlaceholderVerticalAlign
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.BaselineShift
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiRichBlock
import io.github.steeb_k.commune.core.FfiRichBlockKind
import io.github.steeb_k.commune.core.FfiRichInline
import io.github.steeb_k.commune.core.FfiRichMention

/// The document of a text message: the blocks the core built from its
/// formatted body — or its plain body — drawn the way the GTK history
/// draws them. Quotes get a bar down their left, list items their marker,
/// code its monospace box; mentions are pills and links open.
@Composable
fun RichBody(
    state: CommuneState,
    blocks: List<FfiRichBlock>,
    style: TextStyle,
    color: Color,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier) {
        blocks.forEachIndexed { index, block ->
            if (index > 0) Spacer(Modifier.height(4.dp))
            RichBlock(state, block, style, color)
        }
    }
}

@Composable
private fun RichBlock(state: CommuneState, block: FfiRichBlock, style: TextStyle, color: Color) {
    // The row takes the height of its text, so the quote bars can fill it.
    Row(modifier = Modifier.height(IntrinsicSize.Min)) {
        // One bar per quote the block sits in, the GTK `quote` border.
        repeat(block.quoteDepth.toInt()) {
            Box(
                modifier = Modifier
                    .padding(end = 8.dp)
                    .width(3.dp)
                    .fillMaxHeight()
                    .clip(RoundedCornerShape(2.dp))
                    .background(MaterialTheme.colorScheme.outlineVariant),
            )
        }
        // The marker column sits inside the indentation of its list.
        val indent = block.indent.toInt()
        if (indent > 0) {
            Spacer(Modifier.width((12 * (indent - 1)).dp))
            Box(modifier = Modifier.width(22.dp)) {
                block.marker?.let { marker ->
                    Text(marker, style = style, color = color)
                }
            }
        }

        Box(modifier = Modifier.weight(1f, fill = false)) {
            when (val kind = block.kind) {
                is FfiRichBlockKind.Paragraph -> RichLine(state, block, style, color)
                is FfiRichBlockKind.Heading -> RichLine(
                    state,
                    block,
                    headingStyle(kind.level.toInt(), style),
                    color,
                )
                is FfiRichBlockKind.Summary -> RichLine(
                    state,
                    block,
                    style.copy(fontWeight = FontWeight.Bold),
                    color,
                )
                is FfiRichBlockKind.Code -> CodeBlock(kind.text, style)
                is FfiRichBlockKind.Rule -> HorizontalDivider(
                    modifier = Modifier.padding(vertical = 4.dp),
                )
            }
        }
    }
}

/// The type of a heading: the GTK `h1`–`h6` scale, over the body size.
@Composable
private fun headingStyle(level: Int, base: TextStyle): TextStyle {
    val typography = MaterialTheme.typography
    val sized = when (level) {
        1 -> typography.headlineSmall
        2 -> typography.titleLarge
        3 -> typography.titleMedium
        else -> typography.titleSmall
    }
    return base.copy(
        fontSize = sized.fontSize,
        lineHeight = sized.lineHeight,
        fontWeight = FontWeight.Bold,
    )
}

@Composable
private fun CodeBlock(text: String, style: TextStyle) {
    Box(
        modifier = Modifier
            .clip(RoundedCornerShape(6.dp))
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .horizontalScroll(rememberScrollState())
            .padding(horizontal = 8.dp, vertical = 6.dp),
    ) {
        Text(
            text,
            style = style.copy(fontFamily = FontFamily.Monospace),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            softWrap = false,
        )
    }
}

/// A line of runs, as one annotated string with the emoticons placed
/// among the words.
@Composable
private fun RichLine(state: CommuneState, block: FfiRichBlock, style: TextStyle, color: Color) {
    // A message that is nothing but custom emoticons is presented like a
    // sticker: the emoticons large, as pictures of their own.
    if (block.isEmoticonsOnly) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            block.inlines.filterIsInstance<FfiRichInline.Emoticon>().forEach { emoticon ->
                EmoticonImage(state, emoticon.uri, emoticon.body, 64.dp)
                Spacer(Modifier.width(6.dp))
            }
        }
        return
    }

    val context = LocalContext.current
    val primary = MaterialTheme.colorScheme.primary
    val codeBackground = MaterialTheme.colorScheme.surfaceVariant
    val linkStyles = TextLinkStyles(
        style = SpanStyle(color = primary, textDecoration = TextDecoration.Underline),
    )
    val pillStyle = SpanStyle(
        color = primary,
        fontWeight = FontWeight.Bold,
        background = primary.copy(alpha = 0.12f),
    )

    val open: (String) -> Unit = { uri -> openLink(state, context, uri) }

    val inlineContent = mutableMapOf<String, InlineTextContent>()
    val annotated = buildAnnotatedString {
        block.inlines.forEachIndexed { index, inline ->
            when (inline) {
                is FfiRichInline.Text -> {
                    val span = spanStyle(inline, codeBackground)
                    val link = inline.link
                    if (link != null) {
                        withLink(
                            LinkAnnotation.Clickable(
                                tag = link,
                                styles = linkStyles,
                                linkInteractionListener = { open(link) },
                            ),
                        ) {
                            withStyle(span) { append(inline.text) }
                        }
                    } else {
                        withStyle(span) { append(inline.text) }
                    }
                }
                is FfiRichInline.Mention -> {
                    val target = mentionTarget(inline.kind)
                    if (target != null) {
                        withLink(
                            LinkAnnotation.Clickable(
                                tag = target,
                                linkInteractionListener = { open(target) },
                            ),
                        ) {
                            withStyle(pillStyle) { append(" ${inline.name} ") }
                        }
                    } else {
                        withStyle(pillStyle) { append(" ${inline.name} ") }
                    }
                }
                is FfiRichInline.Emoticon -> {
                    val id = "emoticon-$index"
                    inlineContent[id] = InlineTextContent(
                        Placeholder(
                            width = 1.4.em,
                            height = 1.4.em,
                            placeholderVerticalAlign = PlaceholderVerticalAlign.TextCenter,
                        ),
                    ) {
                        EmoticonImage(state, inline.uri, inline.body, 24.dp)
                    }
                    appendInlineContent(id, inline.body.ifEmpty { "�" })
                }
            }
        }
    }

    Text(annotated, style = style, color = color, inlineContent = inlineContent)
}

/// The appearance of a run.
private fun spanStyle(inline: FfiRichInline.Text, codeBackground: Color): SpanStyle {
    val decorations = buildList {
        if (inline.underline) add(TextDecoration.Underline)
        if (inline.strikethrough) add(TextDecoration.LineThrough)
    }
    val shift = when {
        inline.superscript -> BaselineShift.Superscript
        inline.subscript -> BaselineShift.Subscript
        else -> null
    }
    return SpanStyle(
        fontWeight = if (inline.bold) FontWeight.Bold else null,
        fontStyle = if (inline.italic) FontStyle.Italic else null,
        textDecoration = if (decorations.isEmpty()) null else TextDecoration.combine(decorations),
        fontFamily = if (inline.code) FontFamily.Monospace else null,
        background = inline.bgColor?.let(::cssColor) ?: if (inline.code) codeBackground else Color.Unspecified,
        color = inline.color?.let(::cssColor) ?: Color.Unspecified,
        baselineShift = shift,
        fontSize = if (shift != null) 11.sp else androidx.compose.ui.unit.TextUnit.Unspecified,
    )
}

/// A CSS color as the message wrote it — `#rrggbb` or a name — or nothing
/// if it is not one.
private fun cssColor(value: String): Color? = try {
    Color(android.graphics.Color.parseColor(value.trim()))
} catch (_: IllegalArgumentException) {
    null
}

/// The link a mention pill opens: a matrix.to permalink, which the core
/// parses back into what it points at.
private fun mentionTarget(kind: FfiRichMention): String? = when (kind) {
    is FfiRichMention.User -> "https://matrix.to/#/${android.net.Uri.encode(kind.userId)}"
    is FfiRichMention.Room -> {
        val via = kind.via.joinToString("&") { "via=${android.net.Uri.encode(it)}" }
        val query = if (via.isEmpty()) "" else "?$via"
        "https://matrix.to/#/${android.net.Uri.encode(kind.roomIdOrAlias)}$query"
    }
    is FfiRichMention.AtRoom -> null
}

/// Open a link from a message: a Matrix link opens in the app, the way the
/// GTK app routes one — the room, or a dialog for what it points at —
/// and anything else goes to whatever application claims it.
private fun openLink(state: CommuneState, context: android.content.Context, uri: String) {
    if (state.openMatrixLink(uri)) return

    try {
        context.startActivity(
            android.content.Intent(
                android.content.Intent.ACTION_VIEW,
                android.net.Uri.parse(uri),
            ),
        )
    } catch (_: Exception) {
        // Nothing claims the scheme; the text still reads.
    }
}

/// A custom emoticon, fetched from the homeserver like any pack image.
@Composable
private fun EmoticonImage(
    state: CommuneState,
    mxcUri: String,
    body: String,
    size: androidx.compose.ui.unit.Dp,
) {
    var path by remember(mxcUri) { mutableStateOf<String?>(null) }
    LaunchedEffect(mxcUri) { path = state.fetchMxcPath(mxcUri) }

    val current = path
    if (current == null) {
        Box(modifier = Modifier.size(size))
    } else {
        MediaImage(
            current,
            contentDescription = body,
            modifier = Modifier.size(size),
            targetSizePx = 192,
        )
    }
}
