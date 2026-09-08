#!/usr/bin/env python3
"""Build app/src/main/assets/emoji.json from the emojibase dataset.

GTK's emoji chooser (gtk/emoji/convert-emoji.py) is fed the same dataset,
emojibase-data/en/data.json, and keeps for each emoji its codepoints, its
label, its tags and its group; skin-tone variants become one entry with the
modifier slot marked. This produces the same information for the Android
chooser, as one JSON array of

    [emoji, label, [tag, ...], group, [variant, ...]]

where group is the emojibase group number the GTK sections are keyed on
(0 people, 1 body, 3 nature, 4 food, 5 travel, 6 activities, 7 objects,
8 symbols, 9 flags; 2, the tone and hair components, is skipped like the
regional indicators that carry no group at all), and the variants are the
five single skin tones, light to dark, or an empty list.

Usage: gen-emoji-data.py path/to/emojibase-data/en/data.json
"""
import json
import sys

SECTIONS = {0, 1, 3, 4, 5, 6, 7, 8, 9}

with open(sys.argv[1], encoding="utf-8") as f:
    data = json.load(f)

out = []
for entry in data:
    group = entry.get("group")
    if group not in SECTIONS:
        continue
    variants = []
    for skin in entry.get("skins", []):
        tone = skin.get("tone")
        if isinstance(tone, int) and 1 <= tone <= 5:
            variants.append((tone, skin["emoji"]))
    variants.sort()
    out.append([
        entry["emoji"],
        entry["label"],
        entry.get("tags", []),
        group,
        [emoji for _tone, emoji in variants],
    ])

with open("app/src/main/assets/emoji.json", "w", encoding="utf-8") as f:
    json.dump(out, f, ensure_ascii=False, separators=(",", ":"))
print(f"{len(out)} emoji, {sum(1 for e in out if e[4])} with skin tones")
