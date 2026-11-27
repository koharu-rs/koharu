#!/usr/bin/env python3
"""Render the fork planning notes into self-contained HTML pages.

Usage: python3 scripts/build_feature_docs.py

Reads fork-features.md and our-features.md from the repository root and
writes fork-features.html and our-features.html next to them. Each page inlines its
CSS and JavaScript, so it opens straight from disk without a server. Pandoc does
the markdown parsing; the layout, contents rail and callouts are built here.

Re-run after editing either markdown file.
"""

import html
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
COMPILED = "2026-11-20"
DOCS = [
    ("fork-features.md", "fork-features.html", "Fork features", "what the other forks ship"),
    ("our-features.md", "our-features.html", "Our features", "this fork's planned features"),
]


def pandoc(md_path: Path) -> str:
    if not shutil.which("pandoc"):
        sys.exit("pandoc is required: nix shell nixpkgs#pandoc, or install it directly")
    return subprocess.run(
        [
            "pandoc",
            "--from=gfm+smart",
            "--to=html5",
            "--no-highlight",
            "--wrap=none",
            str(md_path),
        ],
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def classify(md_html: str) -> str:
    """Turn label-led paragraphs into callouts and blockquotes into ledes."""

    def repl(m):
        body = m.group(1)
        plain = re.sub(r"<[^>]+>", "", body).strip()
        if plain.startswith(("In a fork:", "Uncertainty:")):
            return f'<p class="callout callout-fork">{body}</p>'
        if plain.startswith("Prior art"):
            return f'<p class="callout callout-prior">{body}</p>'
        if plain.startswith("Open questions"):
            return f'<p class="callout callout-open">{body}</p>'
        return m.group(0)

    md_html = re.sub(r"<p>(.*?)</p>", repl, md_html, flags=re.S)
    return md_html.replace("<blockquote>", '<blockquote class="lede">')


def toc_entries(md_html: str):
    out = []
    for level, hid, text in re.findall(
        r'<h([23]) id="([^"]+)">(.*?)</h[23]>', md_html, flags=re.S
    ):
        text = re.sub(r'<a class="anchor".*?</a>', "", text, flags=re.S)
        text = re.sub(r"<[^>]+>", "", text)
        out.append((int(level), hid, html.unescape(text)))
    return out


CSS = """
:root{
  --paper:#f7f4ef; --ink:#1d1b19; --muted:#6b6459; --line:#e0d9cd;
  --rail:#14130f; --rail-ink:#ded6c7; --rail-dim:#8d8574;
  --accent:#b5462f; --accent-soft:#f0e2da; --gold:#a8813c;
  --code:#efe9df;
  --surface:#fff; --row-hover:#f2ece1; --heading:#241f1a; --intro:#3c372f;
  --lede:#4a4237; --callout-label:#5c5346; --th-line:#cfc6b5;
  --callout-bg:#f1ece2; --callout-line:#e3dbc9; --fork-bg:#efe9dd;
  --prior-bg:#eef1ec; --prior-line:#dde4d8; --open-bg:#f6efe6; --open-line:#ecd9cf;
}
html[data-theme="dark"]{
  --paper:#171410; --ink:#e7e0d2; --muted:#9c937f; --line:#2e2a20;
  --accent:#c9664f; --accent-soft:#2a1c15; --gold:#bb9350; --code:#211d16;
  --surface:#1e1a14; --row-hover:#221e17; --heading:#ece5d4; --intro:#d9d2bf;
  --lede:#c7bfab; --callout-label:#a89d87; --th-line:#3a3425;
  --callout-bg:#1e1b15; --callout-line:#322d22; --fork-bg:#211d14;
  --prior-bg:#1a1e17; --prior-line:#2a3026; --open-bg:#251b14; --open-line:#3b2b20;
}
*{box-sizing:border-box}
html{scroll-behavior:smooth}
body{
  margin:0; background:var(--paper); color:var(--ink);
  font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Inter,Roboto,"Helvetica Neue",Arial,sans-serif;
  font-size:17px; line-height:1.62; -webkit-font-smoothing:antialiased;
}
a{color:var(--accent);text-decoration:none}
a:hover{text-decoration:underline;text-underline-offset:2px}
.layout{display:flex;min-height:100vh;align-items:flex-start}

/* ---- contents rail ---- */
.rail{
  width:296px;flex:0 0 296px;position:sticky;top:0;height:100vh;overflow-y:auto;
  background:var(--rail);color:var(--rail-ink);padding:32px 24px 48px;
  scrollbar-width:thin;scrollbar-color:#3a3527 transparent;
}
.rail .brand{font-size:12px;letter-spacing:.18em;text-transform:uppercase;color:var(--rail-dim);margin-bottom:14px}
.rail h1.doc-title{font-size:21px;line-height:1.25;margin:0 0 6px;color:#fff;font-weight:640;letter-spacing:-.01em}
.rail .sub{color:var(--rail-dim);font-size:13px;margin:0 0 22px}
.docs-switch{display:flex;flex-direction:column;gap:6px;margin:0 0 26px;padding:0 0 22px;border-bottom:1px solid #2a281f}
.docs-switch a{
  display:block;padding:8px 11px;border-radius:7px;color:var(--rail-ink);font-size:13.5px;
  border:1px solid transparent;
}
.docs-switch a:hover{background:#1f1d17;text-decoration:none}
.docs-switch a.active{background:#262319;border-color:#3a3527;color:#fff}
.docs-switch a .tag{display:block;font-size:11px;color:var(--rail-dim);letter-spacing:.04em;margin-top:2px}
.rail .toc-label{font-size:11px;letter-spacing:.16em;text-transform:uppercase;color:var(--rail-dim);margin:0 0 10px}
.rail nav{display:flex;flex-direction:column;gap:1px}
.rail nav a{
  display:block;font-size:13.25px;line-height:1.35;color:#b9b1a1;
  padding:5px 9px;border-left:2px solid transparent;border-radius:0 6px 6px 0;
}
.rail nav a:hover{color:#fff;background:#1d1b16;text-decoration:none}
.rail nav a.lvl3{padding-left:22px;font-size:12.5px;color:#968e7e}
.rail nav a.active{color:#fff;border-left-color:var(--accent);background:#201d16}

/* ---- content ---- */
main{flex:1 1 auto;min-width:0;padding:56px 56px 140px;display:flex;justify-content:center}
article{width:100%;max-width:760px}
article > h1{
  font-size:13px;letter-spacing:.16em;text-transform:uppercase;color:var(--muted);
  font-weight:600;margin:0 0 10px;
}
article > p.intro{font-size:19px;line-height:1.6;color:var(--intro);margin:0 0 22px}
article > p.intro a{text-decoration:underline;text-underline-offset:2px;text-decoration-color:#d8c7b6}
article h2{
  font-size:29px;line-height:1.2;letter-spacing:-.02em;font-weight:660;
  margin:64px 0 18px;padding-top:26px;border-top:1px solid var(--line);
}
article h3{
  font-size:21px;line-height:1.3;letter-spacing:-.012em;font-weight:640;
  margin:44px 0 12px;color:var(--heading);
}
article h4{font-size:17px;margin:28px 0 8px}
article p{margin:0 0 16px}
article ul,article ol{margin:0 0 18px;padding-left:24px}
article li{margin:0 0 7px}
article li::marker{color:var(--accent)}
strong{font-weight:640}
hr{border:0;border-top:1px solid var(--line);margin:56px 0}
code{background:var(--code);padding:2px 5px;border-radius:4px;font-size:.88em;
  font-family:ui-monospace,SFMono-Regular,"JetBrains Mono",Menlo,monospace}
blockquote.lede{
  margin:0 0 22px;padding:2px 0 2px 18px;border-left:3px solid var(--gold);
  font-size:19px;line-height:1.5;color:var(--lede);font-style:italic;
}
blockquote.lede p{margin:0}

/* ---- notes ---- */
.callout{
  margin:0 0 20px;padding:13px 16px 13px 16px;border-radius:8px;
  background:var(--callout-bg);border:1px solid var(--callout-line);font-size:15.5px;line-height:1.55;
}
.callout-fork{background:var(--fork-bg);border-left:3px solid var(--gold)}
.callout-prior{background:var(--prior-bg);border-left:3px solid #7d8f6c;border-color:var(--prior-line)}
.callout-open{background:var(--open-bg);border-left:3px solid var(--accent);border-color:var(--open-line)}
.callout > em:first-child,.callout > strong:first-child{color:var(--callout-label);letter-spacing:.01em}
.callout-open > strong:first-child{color:var(--accent)}

/* ---- table ---- */
table{border-collapse:collapse;width:100%;margin:0 0 26px;font-size:15px}
th,td{text-align:left;padding:10px 12px;border-bottom:1px solid var(--line);vertical-align:top}
th{font-size:12px;letter-spacing:.08em;text-transform:uppercase;color:var(--muted);border-bottom:1px solid var(--th-line)}
tbody tr:hover{background:var(--row-hover)}

/* ---- anchors ---- */
.anchor{opacity:0;margin-left:.35em;font-size:.62em;color:var(--muted);text-decoration:none;transition:opacity .12s}
h2:hover .anchor,h3:hover .anchor{opacity:1}

.top{position:fixed;right:28px;bottom:26px;background:var(--surface);border:1px solid var(--line);
  color:var(--muted);padding:8px 12px;border-radius:999px;font-size:12px;box-shadow:0 4px 14px rgba(30,20,10,.08)}
.top:hover{color:var(--accent);text-decoration:none}
.theme-toggle{bottom:66px;cursor:pointer;font-family:inherit}

@media (max-width:1000px){
  .layout{flex-direction:column}
  .rail{position:relative;width:100%;flex:none;height:auto;padding:24px 20px}
  .rail nav{max-height:none}
  main{padding:36px 22px 90px}
  article h2{margin-top:48px}
}
"""

JS = """
(function(){
  var links = Array.prototype.slice.call(document.querySelectorAll('.rail nav a'));
  var targets = links.map(function(a){ return document.getElementById(decodeURIComponent(a.hash.slice(1))); });
  var byId = {};
  links.forEach(function(a,i){ if(targets[i]) byId[targets[i].id] = a; });
  var current = null;
  function update(){
    var probe = window.scrollY + 140, best = targets[0];
    targets.forEach(function(t){ if(t && t.offsetTop <= probe) best = t; });
    if(best && byId[best.id] && byId[best.id] !== current){
      if(current) current.classList.remove('active');
      current = byId[best.id];
      current.classList.add('active');
    }
  }
  document.addEventListener('scroll', update, {passive:true});
  update();
  var themeRoot = document.documentElement;
  var themeToggle = document.getElementById('themeToggle');
  function syncTheme() {
    themeToggle.textContent = themeRoot.dataset.theme === 'dark' ? 'Light mode' : 'Dark mode';
  }
  if (themeToggle) {
    themeToggle.addEventListener('click', function() {
      var next = themeRoot.dataset.theme === 'dark' ? 'light' : 'dark';
      themeRoot.dataset.theme = next;
      try { localStorage.setItem('koharu-docs-theme', next); } catch (e) {}
      syncTheme();
    });
    syncTheme();
    var scheme = window.matchMedia('(prefers-color-scheme: dark)');
    scheme.addEventListener('change', function(event) {
      var stored = null;
      try { stored = localStorage.getItem('koharu-docs-theme'); } catch (e) {}
      if (stored) return;
      themeRoot.dataset.theme = event.matches ? 'dark' : 'light';
      syncTheme();
    });
  }
})();
"""


THEME_INIT = """
(function(){
  var theme = 'light';
  try {
    var stored = localStorage.getItem('koharu-docs-theme');
    if (stored === 'dark' || stored === 'light') theme = stored;
    else if (window.matchMedia && matchMedia('(prefers-color-scheme: dark)').matches) theme = 'dark';
  } catch (e) {}
  document.documentElement.dataset.theme = theme;
})();
"""


def build(md_name: str, out_name: str, fallback_title: str) -> tuple[Path, int]:
    body = pandoc(ROOT / md_name)
    # The rendered pages link to each other, not to the markdown sources.
    for md, out, *_ in DOCS:
        body = body.replace(f'href="{md}"', f'href="{out}"')

    body = body.replace("<h1", '<h1 class="srch1"')
    body = re.sub(
        r'<h([23]) id="([^"]+)">(.*?)</h\1>',
        lambda m: (
            f'<h{m.group(1)} id="{m.group(2)}">{m.group(3)}'
            f'<a class="anchor" href="#{m.group(2)}">#</a></h{m.group(1)}>'
        ),
        body,
        flags=re.S,
    )

    # The markdown H1 becomes the rail title, so it is not repeated in the body.
    h1 = re.search(r'<h1 class="srch1">(.*?)</h1>', body, flags=re.S)
    doc_title = re.sub(r"<[^>]+>", "", h1.group(1)).strip() if h1 else fallback_title
    body = re.sub(r'<h1 class="srch1">.*?</h1>\s*', "", body, count=1, flags=re.S)

    # Lede styling goes to the first paragraph that is not part of a blockquote.
    parts = re.split(r"(<blockquote>.*?</blockquote>)", body, flags=re.S)
    for i, part in enumerate(parts):
        if not part.startswith("<blockquote") and "<p>" in part:
            parts[i] = part.replace("<p>", '<p class="intro">', 1)
            break
    body = classify("".join(parts))

    toc = toc_entries(body)
    toc_html = "".join(
        f'<a class="lvl{level}" href="#{hid}">{html.escape(text)}</a>'
        for level, hid, text in toc
    )
    switches = "".join(
        f'<a class="{"active" if out == out_name else ""}" href="{out}">'
        f"{label}<span class=\"tag\">{tag}</span></a>"
        for _, out, label, tag in DOCS
    )

    page = f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{html.escape(doc_title)}</title>
<style>{CSS}</style>
<script>{THEME_INIT}</script>
</head>
<body id="top">
<div class="layout">
  <aside class="rail">
    <div class="brand">Koharu fork · planning</div>
    <h1 class="doc-title">{html.escape(doc_title)}</h1>
    <p class="sub">Compiled {COMPILED}</p>
    <div class="docs-switch">{switches}</div>
    <p class="toc-label">Contents</p>
    <nav>{toc_html}</nav>
  </aside>
  <main>
    <article>{body}</article>
  </main>
</div>
<button class="top theme-toggle" id="themeToggle" type="button">Dark mode</button>
<a class="top" href="#top">Back to top</a>
<script>{JS}</script>
</body>
</html>
"""
    out_path = ROOT / out_name
    out_path.write_text(page, encoding="utf-8")
    return out_path, len(toc)


def main() -> None:
    for md, out, title, _ in DOCS:
        if not (ROOT / md).exists():
            sys.exit(f"missing source document: {ROOT / md}")
        path, headings = build(md, out, title)
        print(f"{path}  ({headings} headings)")


if __name__ == "__main__":
    main()
