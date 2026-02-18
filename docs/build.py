#!/usr/bin/env python3
"""Build docs from _layout.html + _pages/*.html sources.

Usage: python3 docs/build.py
"""

import os
import re
import sys

DOCS_DIR = os.path.dirname(os.path.abspath(__file__))
LAYOUT_PATH = os.path.join(DOCS_DIR, '_layout.html')
PAGES_DIR = os.path.join(DOCS_DIR, '_pages')


def parse_page(content):
    """Parse frontmatter, page CSS, body, and page JS from a page source."""
    # Extract frontmatter
    m = re.match(r'^---\n(.*?)\n---\n(.*)$', content, re.DOTALL)
    if not m:
        raise ValueError("page missing --- frontmatter ---")
    meta = {}
    for line in m.group(1).strip().split('\n'):
        key, _, val = line.partition(':')
        meta[key.strip()] = val.strip()

    rest = m.group(2)

    # Extract leading <style>...</style> block
    page_css = ''
    sm = re.match(r'\s*<style>(.*?)</style>\s*(.*)', rest, re.DOTALL)
    if sm:
        page_css = sm.group(1)
        rest = sm.group(2)

    # Extract trailing <script>...</script> block(s)
    page_js = ''
    jm = re.search(r'(\s*<script>.*</script>\s*)$', rest, re.DOTALL)
    if jm:
        page_js = jm.group(1)
        rest = rest[:jm.start()]

    return meta, page_css, rest.strip(), page_js


def build():
    with open(LAYOUT_PATH) as f:
        layout = f.read()

    pages = sorted(f for f in os.listdir(PAGES_DIR) if f.endswith('.html'))
    if not pages:
        print("No pages found in", PAGES_DIR)
        sys.exit(1)

    nav_keys = ['index', 'config', 'channels', 'tools', 'workspace', 'deploy', 'cli']

    for filename in pages:
        with open(os.path.join(PAGES_DIR, filename)) as f:
            source = f.read()

        meta, page_css, body, page_js = parse_page(source)

        # Build nav active attributes
        active = meta.get('active', '')
        nav_replacements = {}
        for key in nav_keys:
            nav_replacements[f'{{{{NAV_{key.upper()}}}}}'] = (
                'class="active"' if key == active else ''
            )

        html = layout
        html = html.replace('{{TITLE}}', meta.get('title', 'oxicrab'))
        html = html.replace('{{DESCRIPTION}}', meta.get('description', ''))
        html = html.replace('{{MAX_WIDTH}}', meta.get('max_width', '820px'))
        html = html.replace('{{PAGE_CSS}}', page_css)
        html = html.replace('{{BODY}}', body)
        html = html.replace('{{PAGE_JS}}', page_js)
        for placeholder, value in nav_replacements.items():
            html = html.replace(placeholder, value)

        out_path = os.path.join(DOCS_DIR, filename)
        with open(out_path, 'w') as f:
            f.write(html)

        print(f"  built {filename}")

    print(f"Done — {len(pages)} pages")


if __name__ == '__main__':
    build()
