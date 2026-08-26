#!/usr/bin/env python3
"""Turn `doc/eyeball-tests.md` into a run sheet you can tick.

The ledger is the source. This only draws it, so the two cannot drift: after
changing the ledger, run this again and republish.

The page keeps what you tick in the browser it was ticked in, and nowhere else.
It can hand back a report and the screenshots attached to failures; the ledger
in the repository is still where a result is recorded for good.
"""

from __future__ import annotations

import hashlib
import html
import json
import re
import subprocess
import sys
from datetime import date
from pathlib import Path

WORKSPACE = Path(__file__).resolve().parent.parent
LEDGER = WORKSPACE / "doc" / "eyeball-tests.md"
OUTPUT = WORKSPACE / "doc" / "eyeball-run.html"
ARTIFACT_URL = "https://claude.ai/code/artifact/9b66f090-2f6d-407e-a802-ea4079773fe9"

# Newest and least-seen first, which is the opposite of the ledger's own order.
# Anything unlisted follows, in the order the ledger has it.
RUN_ORDER = [
    "How to run this",
    "Threads",
    "Spaces",
    "Choosing a space",
    "Reading a room without joining it",
    "Going to a message without leaving the present",
    "Signing up and resetting a password",
    "Presence",
    "Pinned messages",
    "Calls",
    "Server ACLs",
]

# The ledger says outright which checks carry the most weight. Those phrases are
# the author's own, not a guess from the wording.
KEY_PHRASES = (
    "check that matters",
    "worth caring about",
    "most worth doing",
    "check most worth",
    "the check that catches",
    "this is the visible bug",
)


def inline(text: str) -> str:
    """Render the inline markdown the ledger actually uses."""
    out = html.escape(text)
    out = re.sub(r"`([^`]+)`", r"<code>\1</code>", out)
    out = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", out)
    out = re.sub(r"(?<![\w`])_([^_]+)_(?![\w`])", r"<em>\1</em>", out)
    out = re.sub(r"~~([^~]+)~~", r"<del>\1</del>", out)
    return out


def fingerprint(text: str) -> str:
    """A short, stable name for a check, derived from what it says.

    Positional identifiers were a mistake: reordering the ledger moved every
    mark in every browser onto the wrong check. A check keeps its identity as
    long as its wording does, and changing the wording resets it, which is
    right — a rewritten check has not been looked at.
    """
    normalized = re.sub(r"\s+", " ", re.sub(r"[*`_~]", "", text)).strip().lower()
    return "k" + hashlib.sha1(normalized.encode()).hexdigest()[:10]


class Check:
    def __init__(self, section: str, state: str, text: str, destructive: bool):
        self.section = section
        self.done = state == "x"
        self.text = text
        self.destructive = destructive
        self.ident = fingerprint(text)
        lowered = text.lower()
        self.key = any(phrase in lowered for phrase in KEY_PHRASES)

    @property
    def lead(self) -> str:
        """The bolded claim the check opens with.

        A few checks have none, so the first sentence stands in — cut on a
        word, never mid-way through one, since this is what a failure is
        named by in the report.
        """
        match = re.match(r"\*\*(.+?)\*\*", self.text, re.DOTALL)
        if match:
            return re.sub(r"\s+", " ", match.group(1))

        plain = re.sub(r"[*`_]", "", re.sub(r"\s+", " ", self.text)).strip()
        if len(plain) <= 90:
            return plain

        return plain[:90].rsplit(" ", 1)[0] + "\u2026"


class Block:
    """A run of rendered HTML that is not a check."""

    def __init__(self, kind: str, html_text: str):
        self.kind = kind
        self.html = html_text


class Section:
    def __init__(self, title: str, source: str | None):
        self.title = title
        self.source = source
        self.items: list = []

    @property
    def checks(self) -> list[Check]:
        return [item for item in self.items if isinstance(item, Check)]


def parse(markdown: str) -> list[Section]:
    lines = markdown.split("\n")
    sections: list[Section] = []
    current: Section | None = None
    destructive = False
    i = 0

    paragraph: list[str] = []

    def flush_paragraph():
        nonlocal paragraph
        if paragraph and current is not None:
            joined = " ".join(line.strip() for line in paragraph)
            current.items.append(Block("p", f"<p>{inline(joined)}</p>"))
        paragraph = []

    while i < len(lines):
        line = lines[i]

        heading = re.match(r"^## (.+?)(?:\s+—\s+`(.+?)`)?\s*$", line)
        if heading:
            flush_paragraph()
            destructive = False
            current = Section(heading.group(1), heading.group(2))
            sections.append(current)
            i += 1
            continue

        if current is None:
            i += 1
            continue

        subheading = re.match(r"^### (.+)$", line)
        if subheading:
            flush_paragraph()
            # A subsection that takes something away says so on its first line,
            # and everything under it is marked.
            rest = "\n".join(lines[i + 1 : i + 4])
            destructive = bool(re.search(r"^_Destructive", rest, re.MULTILINE))
            current.items.append(
                Block(
                    "h",
                    f'<h3{" class=\'wrecks\'" if destructive else ""}>'
                    f"{inline(subheading.group(1))}</h3>",
                )
            )
            i += 1
            continue

        if line.startswith("```"):
            flush_paragraph()
            i += 1
            body = []
            while i < len(lines) and not lines[i].startswith("```"):
                body.append(lines[i])
                i += 1
            i += 1
            current.items.append(
                Block("pre", f"<pre><code>{html.escape(chr(10).join(body))}</code></pre>")
            )
            continue

        if line.startswith("|"):
            flush_paragraph()
            rows = []
            while i < len(lines) and lines[i].startswith("|"):
                rows.append(lines[i])
                i += 1
            current.items.append(Block("table", render_table(rows)))
            continue

        check = re.match(r"^\* \[([ x!])\] (.*)$", line)
        if check:
            flush_paragraph()
            body = [check.group(2)]
            i += 1
            while i < len(lines) and re.match(r"^ {6,}\S", lines[i]):
                body.append(lines[i].strip())
                i += 1
            current.items.append(
                Check(current.title, check.group(1), " ".join(body), destructive)
            )
            continue

        if line.strip() == "---":
            flush_paragraph()
            i += 1
            continue

        if line.strip() == "":
            flush_paragraph()
            i += 1
            continue

        paragraph.append(line)
        i += 1

    flush_paragraph()
    return sections


def render_table(rows: list[str]) -> str:
    cells = [
        [cell.strip() for cell in row.strip().strip("|").split("|")] for row in rows
    ]
    if len(cells) < 2:
        return ""

    head, body = cells[0], cells[2:]
    out = ['<div class="scroller"><table><thead><tr>']
    out += [f"<th>{inline(cell)}</th>" for cell in head]
    out.append("</tr></thead><tbody>")
    for row in body:
        out.append("<tr>" + "".join(f"<td>{inline(c)}</td>" for c in row) + "</tr>")
    out.append("</tbody></table></div>")
    return "".join(out)


def ordered(sections: list[Section]) -> list[Section]:
    def rank(section: Section) -> int:
        for position, prefix in enumerate(RUN_ORDER):
            if section.title.startswith(prefix):
                return position
        return len(RUN_ORDER)

    return sorted(sections, key=lambda s: (rank(s), sections.index(s)))


LEGACY_REV = "5e34a831"


def legacy_map() -> dict[str, str]:
    """Old positional identifiers to the stable ones that replaced them.

    The first run sheet named checks `c1`…`c193` by their order in the ledger,
    so reordering the ledger moved every mark onto the wrong check. This maps
    what any browser already holds onto the new names, once, by matching the
    text each identifier stood for.
    """
    try:
        previous = subprocess.run(
            ["git", "-C", str(WORKSPACE), "show", f"{LEGACY_REV}:doc/eyeball-tests.md"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except (subprocess.CalledProcessError, FileNotFoundError):
        return {}

    mapping = {}
    index = 0

    for match in re.finditer(r"(?m)^\* \[[ x!]\] (.*(?:\n {6,}\S.*)*)$", previous):
        index += 1
        body = " ".join(line.strip() for line in match.group(1).split("\n"))
        mapping[f"c{index}"] = fingerprint(body)

    return mapping


def commit() -> str:
    try:
        return subprocess.run(
            ["git", "-C", str(WORKSPACE), "log", "-1", "--format=%h"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "unknown"


SCRIPT = r"""
(function () {
  "use strict";

  var KEY = "commune-eyeball-v1";
  var MAX_WIDTH = 1400;
  var state = {};

  function load() {
    try {
      state = JSON.parse(localStorage.getItem(KEY) || "{}") || {};
    } catch (e) {
      state = {};
    }

    if (state.__v === 2) return;

    // The first run sheet named checks by their position in the ledger, so
    // reordering it would have moved every mark onto the wrong check. Carry
    // what this browser holds across to the names that follow the text.
    var map = window.__legacyIds || {};
    var carried = {};

    Object.keys(state).forEach(function (id) {
      if (id.charAt(0) === "_") return;
      var moved = map[id];
      carried[moved || id] = state[id];
    });

    carried.__v = 2;
    state = carried;
    save();
  }

  function save() {
    try {
      localStorage.setItem(KEY, JSON.stringify(state));
      return true;
    } catch (e) {
      warn("There is no room left in this browser to keep another screenshot. " +
           "Save the ones you have, then press Reset.");
      return false;
    }
  }

  function warn(message) {
    var box = document.getElementById("warn");
    if (!box) {
      box = document.createElement("div");
      box.id = "warn";
      box.className = "warn";
      document.querySelector("main").prepend(box);
    }
    box.textContent = message;
  }

  var checks = Array.prototype.slice.call(document.querySelectorAll(".chk"));

  function entry(id) {
    if (!state[id]) state[id] = {};
    return state[id];
  }

  /* ---------- drawing ---------- */

  function paintCheck(node) {
    var id = node.dataset.id;
    var kept = state[id] || {};
    node.dataset.state = kept.mark || "";

    node.querySelectorAll(".ctl button").forEach(function (button) {
      button.classList.toggle("on", kept.mark === button.dataset.act);
      button.setAttribute("aria-pressed", String(kept.mark === button.dataset.act));
    });

    var box = node.querySelector("textarea");
    if (box && box.value !== (kept.note || "")) box.value = kept.note || "";

    paintShot(node, kept);
  }

  function paintShot(node, kept) {
    var wrap = node.querySelector(".shot");
    var drop = wrap.querySelector(".drop");
    var acts = wrap.querySelector(".shot-acts");
    var image = wrap.querySelector("img");

    if (kept.shot) {
      if (!image) {
        image = document.createElement("img");
        wrap.prepend(image);
      }
      image.src = kept.shot;
      image.alt = "Screenshot attached to this check";
      drop.hidden = true;
      acts.innerHTML = "";

      var saveButton = document.createElement("button");
      saveButton.type = "button";
      saveButton.textContent = "Save image";
      saveButton.hidden = !downloads;
      saveButton.addEventListener("click", function () {
        saveImage(node.dataset.id, kept.shot);
      });

      var dropButton = document.createElement("button");
      dropButton.type = "button";
      dropButton.textContent = "Remove";
      dropButton.addEventListener("click", function () {
        delete entry(node.dataset.id).shot;
        save();
        if (image) image.remove();
        paintCheck(node);
      });

      acts.appendChild(saveButton);
      acts.appendChild(dropButton);
    } else {
      if (image) image.remove();
      drop.hidden = false;
      acts.innerHTML = "";
    }
  }

  function paintTotals() {
    var passed = 0, failed = 0, seen = 0;

    checks.forEach(function (node) {
      var mark = (state[node.dataset.id] || {}).mark;
      if (mark === "pass") passed++;
      else if (mark === "fail") failed++;
      if (node.querySelector(".tag.seen")) seen++;
    });

    var total = checks.length;
    document.getElementById("t-pass").textContent = passed;
    document.getElementById("t-fail").textContent = failed;
    document.getElementById("t-left").textContent = total - passed - failed;
    document.getElementById("m-pass").style.width = (passed / total) * 100 + "%";
    document.getElementById("m-fail").style.width = (failed / total) * 100 + "%";

    document.querySelectorAll("section[data-checks]").forEach(function (section) {
      var label = section.querySelector("[data-count]");
      if (!label) return;

      var left = 0;
      section.querySelectorAll(".chk").forEach(function (node) {
        var kept = state[node.dataset.id] || {};
        if (!kept.mark && !node.querySelector(".tag.seen")) left++;
      });

      label.textContent = left ? left + " left" : "all seen";
      label.classList.toggle("clear", left === 0);

      var link = document.querySelector('nav.rail a[data-for="' + section.id + '"]');
      if (link) {
        link.querySelector("b").textContent = left;
        link.classList.toggle("clear", left === 0);
      }
    });
  }

  /* ---------- screenshots ---------- */

  function shrink(file, done) {
    var reader = new FileReader();
    reader.onload = function () {
      var image = new Image();
      image.onload = function () {
        var scale = Math.min(1, MAX_WIDTH / image.width);
        var canvas = document.createElement("canvas");
        canvas.width = Math.round(image.width * scale);
        canvas.height = Math.round(image.height * scale);
        canvas.getContext("2d").drawImage(image, 0, 0, canvas.width, canvas.height);
        done(canvas.toDataURL("image/jpeg", 0.82));
      };
      image.onerror = function () { warn("That file is not an image this browser can read."); };
      image.src = reader.result;
    };
    reader.onerror = function () { warn("That file could not be read."); };
    reader.readAsDataURL(file);
  }

  function attach(node, file) {
    if (!file || !/^image\//.test(file.type)) return;
    shrink(file, function (data) {
      entry(node.dataset.id).shot = data;
      if (save()) paintCheck(node);
    });
  }

  function dataUrlToBlob(url) {
    var parts = url.split(",");
    var binary = atob(parts[1]);
    var bytes = new Uint8Array(binary.length);
    for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return new Blob([bytes], { type: "image/jpeg" });
  }

  function saveImage(id, url) {
    if (!downloads) return;
    downloads.save({ filename: id + ".jpg", data: dataUrlToBlob(url) })
      .catch(function (error) {
        if (error && error.code !== "declined") {
          warn("That image could not be saved: " + (error.message || error.code));
        }
      });
  }

  /* ---------- the report ---------- */

  function report() {
    var lines = ["# Eyeball run — " + document.querySelector(".mark span").textContent,
                 "", "Run on " + new Date().toISOString().slice(0, 10) + ".", ""];
    var passed = 0, failures = [];

    checks.forEach(function (node) {
      var kept = state[node.dataset.id] || {};
      if (kept.mark === "pass") passed++;
      if (kept.mark === "fail") failures.push({ node: node, kept: kept });
    });

    lines.push("**" + passed + " passed, " + failures.length + " failed, " +
               (checks.length - passed - failures.length) + " not looked at.**", "");

    if (!failures.length) {
      lines.push("Nothing failed.");
      return lines.join("\n");
    }

    var section = null;
    lines.push("## What failed", "");

    failures.forEach(function (item) {
      var owner = item.node.closest("section").querySelector("h2").textContent;
      if (owner !== section) {
        section = owner;
        lines.push("### " + owner, "");
      }
      lines.push("* **" + item.node.dataset.lead + "**");
      if (item.kept.note) {
        lines.push("  " + item.kept.note.split("\n").join("\n  "));
      }
      if (item.kept.shot) {
        lines.push("  Screenshot saved as `" + item.node.dataset.id + ".jpg`.");
      }
      lines.push("");
    });

    return lines.join("\n");
  }

  /* ---------- wiring ---------- */

  checks.forEach(function (node) {
    node.querySelectorAll(".ctl button").forEach(function (button) {
      button.addEventListener("click", function () {
        var kept = entry(node.dataset.id);
        kept.mark = kept.mark === button.dataset.act ? "" : button.dataset.act;
        save();
        paintCheck(node);
        paintTotals();
      });
    });

    var box = node.querySelector("textarea");
    var timer = null;
    box.addEventListener("input", function () {
      clearTimeout(timer);
      timer = setTimeout(function () {
        entry(node.dataset.id).note = box.value;
        save();
      }, 400);
    });

    var drop = node.querySelector(".drop");
    drop.addEventListener("click", function () {
      var picker = document.createElement("input");
      picker.type = "file";
      picker.accept = "image/*";
      picker.addEventListener("change", function () {
        attach(node, picker.files && picker.files[0]);
      });
      picker.click();
    });
    drop.addEventListener("keydown", function (event) {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        drop.click();
      }
    });
    ["dragenter", "dragover"].forEach(function (name) {
      drop.addEventListener(name, function (event) {
        event.preventDefault();
        drop.classList.add("over");
      });
    });
    ["dragleave", "drop"].forEach(function (name) {
      drop.addEventListener(name, function () { drop.classList.remove("over"); });
    });
    drop.addEventListener("drop", function (event) {
      event.preventDefault();
      attach(node, event.dataTransfer.files && event.dataTransfer.files[0]);
    });
    drop.addEventListener("paste", function (event) {
      var items = event.clipboardData && event.clipboardData.files;
      if (items && items.length) {
        event.preventDefault();
        attach(node, items[0]);
      }
    });
  });

  document.addEventListener("paste", function (event) {
    var active = document.activeElement;
    var node = active && active.closest ? active.closest(".chk") : null;
    if (!node || node.dataset.state !== "fail") return;
    var files = event.clipboardData && event.clipboardData.files;
    if (files && files.length) {
      event.preventDefault();
      attach(node, files[0]);
    }
  });

  document.querySelectorAll("section > header").forEach(function (header) {
    header.addEventListener("click", function () {
      var section = header.parentElement;
      section.dataset.open = section.dataset.open === "1" ? "0" : "1";
    });
  });

  document.getElementById("a-copy").addEventListener("click", function () {
    var text = report();
    navigator.clipboard.writeText(text).then(function () {
      var button = document.getElementById("a-copy");
      button.textContent = "Copied";
      setTimeout(function () { button.textContent = "Copy report"; }, 1600);
    }, function () {
      warn("This browser would not take the report. Save it as a file instead.");
    });
  });

  document.getElementById("a-reset").addEventListener("click", function () {
    if (!confirm("Forget every mark, note and screenshot on this page?")) return;
    state = {};
    save();
    checks.forEach(paintCheck);
    paintTotals();
  });

  /* ---------- capability ---------- */

  var downloads = null;

  if (window.claude && window.claude.use) {
    window.claude.use("downloads").then(function (namespace) {
      if (!namespace) return;
      downloads = namespace;

      var button = document.getElementById("a-save");
      button.hidden = false;
      button.addEventListener("click", function () {
        downloads.save({ filename: "eyeball-run.md", data: report() })
          .catch(function (error) {
            if (error && error.code !== "declined") {
              warn("That report could not be saved: " + (error.message || error.code));
            }
          });
      });

      checks.forEach(paintCheck);
    }, function () {});
  }

  load();
  checks.forEach(paintCheck);
  paintTotals();
})();
"""

STYLE = """
:root {
  --ground:#f2f4f5; --surface:#ffffff; --surface-alt:#e9edee; --raised:#ffffff;
  --line:#d3dadc; --line-soft:#e3e8e9;
  --ink:#131a1c; --ink-mid:#4a585c; --ink-soft:#74848a;
  --accent:#0d6f6c; --accent-soft:#d3e8e6;
  --pass:#34694a; --pass-soft:#d9e8de;
  --fail:#a93a2a; --fail-soft:#f3ddd8;
  --key:#8d6208;  --key-soft:#f2e5c8;
  --f-display:"IBM Plex Sans Condensed","Helvetica Neue",Arial,sans-serif;
  --f-body:"IBM Plex Sans","Helvetica Neue",Arial,sans-serif;
  --f-mono:"IBM Plex Mono",ui-monospace,"SFMono-Regular",Menlo,monospace;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --ground:#10161a; --surface:#171e22; --surface-alt:#1e262b; --raised:#1c2429;
    --line:#2c373d; --line-soft:#232c31;
    --ink:#e2e8ea; --ink-mid:#a3b1b6; --ink-soft:#7e8f95;
    --accent:#52bdb5; --accent-soft:#1c3634;
    --pass:#79b491; --pass-soft:#1b2b22;
    --fail:#e08573; --fail-soft:#35211d;
    --key:#d8ab48;  --key-soft:#322817;
  }
}
:root[data-theme="dark"] {
  --ground:#10161a; --surface:#171e22; --surface-alt:#1e262b; --raised:#1c2429;
  --line:#2c373d; --line-soft:#232c31;
  --ink:#e2e8ea; --ink-mid:#a3b1b6; --ink-soft:#7e8f95;
  --accent:#52bdb5; --accent-soft:#1c3634;
  --pass:#79b491; --pass-soft:#1b2b22;
  --fail:#e08573; --fail-soft:#35211d;
  --key:#d8ab48;  --key-soft:#322817;
}

* { box-sizing:border-box; }
body {
  margin:0; background:var(--ground); color:var(--ink);
  font-family:var(--f-body); font-size:16px; line-height:1.6;
  -webkit-font-smoothing:antialiased;
}
code { font-family:var(--f-mono); font-size:.86em;
       background:var(--surface-alt); padding:1px 5px; border-radius:2px; }
pre { font-family:var(--f-mono); font-size:.82rem; line-height:1.6;
      background:var(--surface-alt); border:1px solid var(--line);
      padding:12px 14px; overflow-x:auto; margin:0; }
pre code { background:none; padding:0; font-size:1em; }
a { color:var(--accent); }
:focus-visible { outline:2px solid var(--accent); outline-offset:2px; }

/* ---------- status bar ---------- */
.bar {
  position:sticky; top:0; z-index:10; background:var(--surface);
  border-bottom:1px solid var(--line);
}
.bar-in {
  max-width:1180px; margin:0 auto; padding:14px 24px;
  display:flex; align-items:center; gap:20px; flex-wrap:wrap;
}
.mark { display:flex; flex-direction:column; gap:1px; margin-right:auto; }
.mark b {
  font-family:var(--f-display); font-weight:700; font-size:1.25rem;
  letter-spacing:-.01em; line-height:1;
}
.mark span {
  font-family:var(--f-mono); font-size:10.5px; letter-spacing:.12em;
  text-transform:uppercase; color:var(--ink-soft);
}
.tally { display:flex; gap:16px; font-variant-numeric:tabular-nums; }
.tally div { display:flex; flex-direction:column; align-items:flex-end; gap:1px; }
.tally b { font-family:var(--f-display); font-size:1.5rem; font-weight:700; line-height:1; }
.tally span { font-family:var(--f-mono); font-size:10px; letter-spacing:.1em;
              text-transform:uppercase; color:var(--ink-soft); }
.tally .n-pass b { color:var(--pass); }
.tally .n-fail b { color:var(--fail); }
.meter { flex-basis:100%; height:4px; background:var(--line-soft); display:flex; }
.meter i { display:block; height:100%; transition:width .2s ease; }
.meter .m-pass { background:var(--pass); }
.meter .m-fail { background:var(--fail); }
.acts { display:flex; gap:8px; flex-wrap:wrap; }

button {
  font-family:var(--f-body); font-size:.8125rem; color:var(--ink);
  background:var(--surface-alt); border:1px solid var(--line);
  padding:6px 12px; border-radius:3px; cursor:pointer; line-height:1.4;
}
button:hover { border-color:var(--ink-soft); }
/* The accent is dark on a light ground and light on a dark one, so the
   surface token is already the readable text colour against it in both. */
button.primary { background:var(--accent); border-color:var(--accent); color:var(--surface); }
button.primary:hover { border-color:var(--accent); filter:brightness(1.08); }

/* ---------- page ---------- */
.wrap { max-width:1180px; margin:0 auto; padding:32px 24px 96px;
        display:grid; grid-template-columns:200px 1fr; gap:40px; align-items:start; }
@media (max-width:900px) { .wrap { grid-template-columns:1fr; gap:24px; } nav.rail { display:none; } }

nav.rail { position:sticky; top:88px; display:flex; flex-direction:column; gap:2px; }
nav.rail a {
  font-family:var(--f-mono); font-size:11.5px; letter-spacing:.03em;
  color:var(--ink-mid); text-decoration:none; padding:5px 8px; border-radius:2px;
  display:flex; justify-content:space-between; gap:8px; border-left:2px solid transparent;
}
nav.rail a:hover { background:var(--surface-alt); color:var(--ink); }
nav.rail a b { font-weight:500; color:var(--ink-soft); font-variant-numeric:tabular-nums; }
nav.rail a.clear { border-left-color:var(--pass); }
nav.rail a.clear b { color:var(--pass); }

.lede { margin:0 0 28px; color:var(--ink-mid); max-width:64ch; }
.lede strong { color:var(--ink); }

section { margin-bottom:14px; border:1px solid var(--line); background:var(--surface); }
section > header {
  display:flex; align-items:baseline; gap:12px; padding:14px 20px;
  border-bottom:1px solid var(--line-soft); cursor:pointer; user-select:none;
}
section > header h2 {
  font-family:var(--f-display); font-weight:600; font-size:1.375rem;
  letter-spacing:-.01em; margin:0; flex:1; text-wrap:balance;
}
section > header .src { font-family:var(--f-mono); font-size:11px; color:var(--ink-soft); }
section > header .cnt {
  font-family:var(--f-mono); font-size:11px; letter-spacing:.06em;
  padding:3px 8px; border-radius:2px; background:var(--surface-alt);
  color:var(--ink-mid); font-variant-numeric:tabular-nums; white-space:nowrap;
}
section > header .cnt.clear { background:var(--pass-soft); color:var(--pass); }
section[data-open="0"] > .body { display:none; }
.body { padding:6px 20px 20px; }
.body h3 {
  font-family:var(--f-display); font-weight:600; font-size:1.0625rem;
  margin:22px 0 8px; color:var(--ink);
}
.body p { margin:10px 0; color:var(--ink-mid); max-width:68ch; }
.scroller { overflow-x:auto; margin:12px 0; }
table { border-collapse:collapse; width:100%; font-size:.875rem; }
th, td { text-align:left; padding:7px 12px; border-bottom:1px solid var(--line-soft);
         vertical-align:top; }
th { font-family:var(--f-mono); font-size:10.5px; letter-spacing:.09em;
     text-transform:uppercase; color:var(--ink-mid); }

/* ---------- a check ---------- */
.chk { display:grid; grid-template-columns:auto 1fr; gap:10px 12px;
       padding:10px 0; border-bottom:1px solid var(--line-soft); }
.chk:last-child { border-bottom:none; }
.chk .ctl { display:flex; gap:4px; padding-top:2px; }
.chk .ctl button {
  width:26px; height:26px; padding:0; display:grid; place-items:center;
  font-family:var(--f-mono); font-size:13px; line-height:1; background:var(--surface);
}
.chk .ctl button.on[data-act="pass"] { background:var(--pass); border-color:var(--pass); color:var(--surface); }
.chk .ctl button.on[data-act="fail"] { background:var(--fail); border-color:var(--fail); color:var(--surface); }
.chk .txt { min-width:0; }
.chk[data-state="pass"] .txt { color:var(--ink-soft); }
.chk[data-state="pass"] .txt strong { color:var(--ink-mid); }
.chk .tag {
  display:inline-block; font-family:var(--f-mono); font-size:9.5px; font-weight:600;
  letter-spacing:.09em; text-transform:uppercase; padding:2px 6px; border-radius:2px;
  margin-right:7px; vertical-align:1px;
}
.chk .tag.key { background:var(--key-soft); color:var(--key); }
.chk .tag.wrecks { background:var(--fail-soft); color:var(--fail); }
.body h3.wrecks::after {
  content:"takes things away"; font-family:var(--f-mono); font-size:10px;
  letter-spacing:.09em; text-transform:uppercase; color:var(--fail);
  background:var(--fail-soft); padding:3px 7px; border-radius:2px;
  margin-left:10px; vertical-align:3px;
}
.chk .tag.seen { background:var(--pass-soft); color:var(--pass); }
.chk .extra { grid-column:2; display:none; flex-direction:column; gap:8px; padding-bottom:4px; }
.chk[data-state="fail"] .extra { display:flex; }
.chk textarea {
  font-family:var(--f-body); font-size:.875rem; color:var(--ink);
  background:var(--surface-alt); border:1px solid var(--line); border-radius:3px;
  padding:8px 10px; resize:vertical; min-height:60px; width:100%;
}
.shot { display:flex; align-items:flex-start; gap:10px; flex-wrap:wrap; }
.drop {
  flex:1; min-width:200px; border:1px dashed var(--line); border-radius:3px;
  padding:10px 12px; font-size:.8125rem; color:var(--ink-soft); text-align:center;
  cursor:pointer; background:var(--surface-alt);
}
.drop.over { border-color:var(--accent); color:var(--accent); }
.shot img { max-width:280px; max-height:180px; border:1px solid var(--line); border-radius:3px; display:block; }
.shot .shot-acts { display:flex; flex-direction:column; gap:6px; }
.note { font-size:.8125rem; color:var(--ink-soft); }
.warn { background:var(--key-soft); color:var(--key); padding:8px 12px;
        border-radius:3px; font-size:.8125rem; margin:12px 0; }
@media (prefers-reduced-motion:reduce) { * { transition:none !important; } }
"""


def render_check(check: Check) -> str:
    tags = ""
    if check.destructive:
        tags += '<span class="tag wrecks" title="Takes away what the checks above are looking at — leave it until last">Destructive</span>'
    if check.key:
        tags += '<span class="tag key" title="The ledger names this one as carrying the most weight">Key</span>'
    if check.done:
        tags += '<span class="tag seen">Seen</span>'

    return f"""<div class="chk" data-id="{check.ident}" data-state="" data-lead="{html.escape(check.lead, quote=True)}">
<div class="ctl">
<button type="button" data-act="pass" title="Behaves as described" aria-label="Passed">&#10003;</button>
<button type="button" data-act="fail" title="Does not behave as described" aria-label="Failed">&#10007;</button>
</div>
<div class="txt">{tags}{inline(check.text)}</div>
<div class="extra">
<textarea placeholder="What happened? Not what should have."></textarea>
<div class="shot">
<div class="drop" tabindex="0" role="button">Drop, paste or click for a screenshot</div>
<div class="shot-acts"></div>
</div>
</div>
</div>"""


def render(sections: list[Section], head: str) -> str:
    rail, body = [], []

    for number, section in enumerate(sections):
        checks = section.checks
        ident = f"s{number}"
        open_count = sum(1 for c in checks if not c.done)

        if checks:
            rail.append(
                f'<a href="#{ident}" data-for="{ident}">'
                f"<span>{html.escape(section.title)}</span><b>{open_count}</b></a>"
            )

        pieces = []
        for item in section.items:
            pieces.append(
                render_check(item) if isinstance(item, Check) else item.html
            )

        source = (
            f'<span class="src">{html.escape(section.source)}</span>'
            if section.source
            else ""
        )
        count = (
            f'<span class="cnt" data-count>{open_count} left</span>' if checks else ""
        )

        body.append(
            f'<section id="{ident}" data-open="1" data-checks="{len(checks)}">'
            f"<header>{f'<h2>{html.escape(section.title)}</h2>'}{source}{count}</header>"
            f'<div class="body">{"".join(pieces)}</div>'
            "</section>"
        )

    return f"""<!-- Published at {ARTIFACT_URL}
     Generated by doc/eyeball-page.py from doc/eyeball-tests.md — do not edit
     by hand. Regenerate and republish to that URL after changing the ledger. -->
<title>Commune Eyeball Run</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500;600&family=IBM+Plex+Sans+Condensed:wght@600;700&family=IBM+Plex+Sans:ital,wght@0,400;0,500;0,600;1,400&display=swap">
<style>{STYLE}</style>

<div class="bar"><div class="bar-in">
<div class="mark"><b>Eyeball Run</b><span>Commune &middot; {head}</span></div>
<div class="tally">
<div class="n-pass"><b id="t-pass">0</b><span>Passed</span></div>
<div class="n-fail"><b id="t-fail">0</b><span>Failed</span></div>
<div><b id="t-left">0</b><span>Left</span></div>
</div>
<div class="acts">
<button type="button" id="a-copy">Copy report</button>
<button type="button" id="a-save" class="primary" hidden>Save report</button>
<button type="button" id="a-reset">Reset</button>
</div>
<div class="meter"><i class="m-pass" id="m-pass" style="width:0"></i><i class="m-fail" id="m-fail" style="width:0"></i></div>
</div></div>

<div class="wrap">
<nav class="rail">{"".join(rail)}</nav>
<main>
<p class="lede">Every check here passes the compiler, clippy, the tests and all fifteen
of <code>checks-bin</code>, and none of them can judge it. Work top to bottom &mdash;
the sections are ordered newest first, because the code nobody has looked at is the
code most likely to be wrong. <strong>What you tick stays in this browser and nowhere
else.</strong> Mark a failure and you get somewhere to say what happened and to attach
a screenshot; <em>Copy report</em> or <em>Save report</em> then gathers them up.</p>
{"".join(body)}
</main>
</div>
<script>window.__legacyIds = {json.dumps(legacy_map())};</script>
<script>{SCRIPT}</script>"""


def main() -> int:
    sections = ordered(parse(LEDGER.read_text(encoding="utf-8")))
    # Windows Python would otherwise write the locale codepage with CRLF, and
    # a page that is not valid UTF-8 stops `typos`' ignore-patterns matching.
    OUTPUT.write_text(render(sections, commit()), encoding="utf-8", newline="\n")

    total = sum(len(section.checks) for section in sections)
    left = sum(1 for s in sections for c in s.checks if not c.done)
    print(f"{OUTPUT.relative_to(WORKSPACE)}: {total} checks, {left} not looked at")

    return 0


if __name__ == "__main__":
    sys.exit(main())
