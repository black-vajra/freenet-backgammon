#!/usr/bin/env python3
"""Inspect four known old Backgammon websites; optionally retire them in place."""
import argparse
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import subprocess
import tempfile

CURRENT = '38czsTGsDqUtMyd1TzW4BwX3t1rGd12QuNen7mbn2jcb'
TARGETS = [
    ('freenet-backgammon', '9ZFiVUxL2yJuHnW2Dum3nhP7znRRGSHJjHFFqzmPbcWo'),
    ('freenet-backgammon-durable-preview', 'JD5zDvZKVtyC9GgjswEGbbhECroXDhUC6Fa9kPRTEng6'),
    ('backgammon-cleanup-20261004-200306', 'Hpzer5QCkSSPPgjcJgynE7hzx17fyhqsgk8zbG561v3B'),
    ('freenet-backgammon-recovery-preview-20261004', 'Hk2eb7RycLPakXnUJvESX7QwqUNXHBz9oX2E7RKM9Eeb'),
]

class Page(HTMLParser):
    def __init__(self):
        super().__init__()
        self.links = []
        self.scripts = 0
        self.embeds = 0
        self.assets = []
        self.headings = []
        self.heading_depth = 0
    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag in ('iframe', 'object', 'embed'): self.embeds += 1
        if tag == 'script':
            self.scripts += 1
            if attrs.get('src'): self.assets.append(attrs['src'])
        if tag == 'a' and attrs.get('href'): self.links.append(attrs['href'])
        if tag in ('h1', 'title'): self.heading_depth += 1
        if tag == 'link' and attrs.get('href'): self.assets.append(attrs['href'])
    def handle_endtag(self, tag):
        if tag in ('h1', 'title'): self.heading_depth = max(0, self.heading_depth - 1)
    def handle_data(self, data):
        if self.heading_depth: self.headings.append(data.strip())
    def is_retired(self):
        heading = ' '.join(self.headings).lower()
        notice = ('retired' in heading or 'not the official' in heading
                  or 'not the current' in heading or 'no longer current' in heading)
        return self.scripts == 0 and self.embeds == 0 and notice and '/v1/contract/web/' + CURRENT + '/' in self.links


def inspect_page(path):
    if path.stat().st_size > 2 * 1024 * 1024:
        raise RuntimeError('Page exceeds inspection size limit: ' + str(path))
    page = Page()
    page.feed(path.read_text(encoding='utf-8'))
    return page


def notice(contract):
    return f'''<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Freenet Backgammon — Retired website</title>
<style>
:root{{color-scheme:dark}}body{{margin:0;background:#170e09;color:#f8efdc;font:1.1rem/1.6 system-ui,sans-serif}}
main{{max-width:44rem;margin:10vh auto;padding:clamp(1.2rem,4vw,3rem)}}
h1{{font-size:clamp(1.7rem,5vw,2.4rem);line-height:1.2}}
a{{display:inline-block;margin:.7rem 0;padding:.8rem 1.2rem;background:#f0cf92;color:#20140b;border-radius:.5rem;font-weight:700}}
a:focus-visible{{outline:3px solid white;outline-offset:4px}}
small{{display:block;overflow-wrap:anywhere;color:#d6c2a7}}
</style></head><body><main>
<h1>This is not the official Backgammon contract</h1>
<p>This website address is retired. Please use the current Freenet Backgammon website to play.</p>
<a href="/v1/contract/web/{CURRENT}/">Open current Freenet Backgammon</a>
<p>The link opens through the Freenet node you are using now.</p>
<small>Retired website: {contract}</small>
<small>Current website: {CURRENT}</small>
</main></body></html>
'''


def execute(argv, log, timeout=90):
    print('+ ' + ' '.join(argv), flush=True)
    result = subprocess.run(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            text=True, timeout=timeout)
    log.write_text(result.stdout)
    print(result.stdout, end='' if result.stdout.endswith('\n') else '\n', flush=True)
    if result.returncode:
        raise RuntimeError(f'Command failed ({result.returncode}); see {log}')
    return result.stdout


def fetch(contract, folder, label):
    page = folder / (label + '.html')
    execute(['curl', '--noproxy', '*', '--fail', '--silent', '--show-error',
             '--max-time', '60', '-H', 'Cache-Control: no-cache',
             '--dump-header', str(folder / (label + '.headers')),
             '--output', str(page),
             'http://127.0.0.1:7509/v1/contract/web/' + contract + '/?__sandbox=1'],
            folder / (label + '-fetch.log'), timeout=70)
    parsed = inspect_page(page)
    if parsed.embeds > 0 and 'Freenet' in parsed.headings:
        raise RuntimeError('Gateway shell returned instead of the app; retirement status is unknown.')
    return parsed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true', help='Publish notices after all four inspections succeed')
    args = parser.parse_args()
    downloads = Path.home() / 'Downloads'
    if not downloads.is_dir(): raise RuntimeError('Downloads directory is missing.')
    report = Path(tempfile.mkdtemp(prefix='backgammon-old-site-audit-', dir=downloads))
    print('Evidence directory: ' + str(report), flush=True)
    rows = []
    try:
        listing = execute(['fdev', 'network', 'website', 'list'], report / 'website-keys.txt')
        mapping = {}
        for line in listing.splitlines():
            parts = line.split()
            if len(parts) == 2:
                if parts[0] in mapping: raise RuntimeError('Duplicate key name in listing: ' + parts[0])
                mapping[parts[0]] = parts[1]
        if mapping.get('backgammon-pending-20261004-204856') != CURRENT:
            raise RuntimeError('Current website key differs. Nothing published.')
        for key, contract in TARGETS:
            if contract == CURRENT or mapping.get(key) != contract:
                raise RuntimeError('Historical key mapping differs: ' + key + '. Nothing published.')
        # Inspect and snapshot ALL four before publishing to any of them.
        for key, contract in TARGETS:
            folder = report / key
            folder.mkdir()
            state = folder / 'before.cbor'
            execute(['fdev', 'network', '-p', '7509', 'execute', 'get', '--timeout', '60',
                     '--output', str(state), contract], folder / 'before-get.log', timeout=75)
            if not state.is_file() or not state.stat().st_size:
                raise RuntimeError('Empty or missing prepublication contract capture: ' + key)
            digest = hashlib.sha256(state.read_bytes()).hexdigest()
            (folder / 'before.sha256').write_text(digest + '  before.cbor\n')
            page = fetch(contract, folder, 'before')
            candidate = folder / 'retirement-site'
            candidate.mkdir()
            (candidate / 'index.html').write_text(notice(contract), encoding='utf-8')
            row = dict(key=key, contract=contract, before_bytes=state.stat().st_size,
                       before_sha256=digest, before_headings=page.headings,
                       before_scripts=page.scripts, before_assets=page.assets,
                       already_retired=page.is_retired(), status='inspection complete')
            rows.append(row)
            print(f'INSPECTED {key}: scripts={page.scripts}; already retired={page.is_retired()}; headings={page.headings}', flush=True)
        (report / 'inspection.json').write_text(json.dumps(rows, indent=2) + '\n')
        if not args.apply:
            print('Inspection only: nothing published. Run with --apply to perform the authorized retirement.')
            return
        for row in rows:
            folder = report / row['key']
            if row['already_retired']:
                row['status'] = 'already a retirement notice; unchanged'
                continue
            row['status'] = 'publication attempted; confirmation unknown'
            (report / 'summary.json').write_text(json.dumps(rows, indent=2) + '\n')
            execute(['fdev', 'network', 'website', 'publish', '--key', row['key'],
                     '--timeout', '300', str(folder / 'retirement-site')],
                    folder / 'publish.log', timeout=330)
            row['status'] = 'notice published; serving-node verification pending'
            (report / 'summary.json').write_text(json.dumps(rows, indent=2) + '\n')
        # A stale served copy is recorded, without publishing again or dropping other targets.
        for row in rows:
            if row['already_retired']: continue
            try:
                page = fetch(row['contract'], report / row['key'], 'after')
                if page.is_retired(): row['status'] = 'notice published and served on Pots'
                else: row['status'] = 'notice published; Pots still serves an unconfirmed/stale page'
            except (RuntimeError, subprocess.TimeoutExpired, OSError, UnicodeError) as exc:
                row['status'] = 'notice published; verification failed: ' + str(exc)
    except (RuntimeError, subprocess.TimeoutExpired, OSError, UnicodeError) as exc:
        (report / 'ERROR.txt').write_text(str(exc) + '\n')
        print('STOPPED: ' + str(exc), flush=True)
        raise SystemExit(1)
    finally:
        (report / 'summary.json').write_text(json.dumps(rows, indent=2) + '\n')
        print('\nEvidence retained: ' + str(report), flush=True)
        for row in rows: print(row['key'] + ': ' + row['status'], flush=True)
    print('No Atlas edits were made. Verify these notices on EliteBook separately.', flush=True)

if __name__ == '__main__':
    main()
