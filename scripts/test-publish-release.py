"""Exercise the real publisher against stateful gh/curl doubles, without credentials."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
TAG = "v0.9.1"
TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin",
           "aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"]
NAMES = ["Cowchat-0.9.1.dmg"] + [f"cowchat-{TAG}-{t}.tar.gz" for t in TARGETS]
# Each CLI invocation loads/saves a fake remote. Binary fixture contents are
# deliberately small ASCII strings so the JSON state remains inspectable.
STUB = r'''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
p = Path(os.environ['FAKE_STATE'])
s = json.loads(p.read_text())
a = sys.argv[1:]
command = Path(sys.argv[0]).name
s['calls'].append([command] + a)
def save(): p.write_text(json.dumps(s))
def die(): save(); sys.exit(1)
def option(k): return a[a.index(k)+1]
def release(): return s['release']
def assets(): return release()['assets']
def content(name): return Path(os.environ['FAKE_DIST'], name).read_text()
def add(name):
    assets().append({'id': 10 + len(assets()), 'name': name, 'state': 'uploaded', 'bytes': content(name)})
if command == 'sleep':
    save(); sys.exit()
if command == 'curl':
    from urllib.parse import urlsplit
    if s.get('probe_fail'): die()
    urls = [x for x in a if x.startswith(('https://', 'http://'))]
    if len(urls) != 1: die()
    url = urlsplit(urls[0])
    if url.scheme != 'https' or url.netloc != 'github.com' or url.query or url.fragment: die()
    parts = url.path.split('/')
    if len(parts) != 7 or parts[:4] != ['', 'cowboyinc', 'cowchat', 'releases']: die()
    is_latest = parts[4:6] == ['latest', 'download']
    if not is_latest and parts[4] != 'download': die()
    if is_latest and s.get('change_latest'):
        next_release = s.get('next_release')
        if not next_release:
            save()
            raise AssertionError('change_latest requires a next_release fixture')
        s['other_releases'] = [next_release]
        s['latest'] = next_release['tag_name']
    tag = s['latest'] if is_latest else parts[5]
    candidates = ([release()] if release() else []) + s.get('other_releases', [])
    remote = next((r for r in candidates if r['tag_name'] == tag), None)
    if not remote or remote['draft']: die()
    if is_latest and remote['prerelease']: die()
    # Faults occur only once public serving begins, after authenticated checks.
    if s.get('public_missing_alias'):
        remote['assets'] = [x for x in remote['assets'] if x['name'] != 'Cowchat.dmg']
    asset = next((x for x in remote['assets'] if x['name'] == parts[6]), None)
    if not asset or asset['state'] != 'uploaded': die()
    payload = asset['bytes']
    if s.get('public_wrong_bytes') == ('latest' if is_latest else 'tag'):
        payload = 'wrong-public-bytes'
    Path(option('-o')).write_text(payload)
    save(); sys.exit()
if a[0] == 'api':
    endpoint = next(x for x in a if x.startswith('repos/'))
    if s.get('api_fail'): die()
    if '/releases/assets/' in endpoint:
        asset = next(x for x in assets() if x['id'] == int(endpoint.rsplit('/', 1)[1]))
        if s.get('download_fail'): die()
        sys.stdout.write(asset['bytes'])
    elif '/assets?' in endpoint:
        print(json.dumps([assets()]))
    elif endpoint.endswith('/latest'):
        if s.get('latest_fail') or (s.get('latest_fail_after_publish') and release() and not release()['draft']): die()
        print(s['latest'])
    else:
        rows = ([release()] if release() else []) + s.get('other_releases', [])
        if s['latest'] and not any(x['tag_name'] == s['latest'] for x in rows):
            rows.append({'id': 2, 'tag_name': s['latest'], 'draft': False, 'prerelease': False, 'assets': []})
        print(json.dumps({'unexpected': True} if s.get('wrong_shape') else [rows]))
elif a[:2] == ['release', 'create']:
    s['release'] = {'id': 1, 'tag_name': a[2], 'draft': True, 'prerelease': False, 'assets': []}
elif a[:2] == ['release', 'upload']:
    name = Path(a[3]).name
    assert '--clobber' not in a
    if s.get('upload_fail'): die()
    add(name)
    if s.get('conflict'): die()
    if s.get('corrupt_upload'): assets()[-1]['bytes'] = 'corrupted'
elif a[:2] == ['release', 'edit']:
    if '--draft=false' in a: release()['draft'] = False
    if '--latest' in a or '--latest=true' in a: s['latest'] = a[2]
else:
    raise AssertionError(a)
save()
'''


class PublisherTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name)
        self.dist = self.base / 'dist'
        self.dist.mkdir()
        for name in NAMES:
            (self.dist / name).write_text(f'dmg-{TAG}' if name.endswith('.dmg') else name)
        self.state = self.base / 'state.json'
        self.data = {'release': None, 'latest': '', 'calls': []}
        bindir = self.base / 'bin'
        bindir.mkdir()
        for name in ['gh', 'curl', 'sleep']:
            p = bindir / name
            p.write_text(STUB)
            p.chmod(0o755)
        self.env = {**os.environ, 'PATH': str(bindir) + os.pathsep + os.environ['PATH'],
                    'FAKE_STATE': str(self.state), 'FAKE_DIST': str(self.dist),
                    'GITHUB_STEP_SUMMARY': str(self.base / 'summary')}
        for key in ['GH_TOKEN', 'GITHUB_TOKEN']:
            self.env.pop(key, None)

    def existing(self, draft=False, alias=True):
        names = NAMES + (['Cowchat.dmg'] if alias else [])
        self.data['release'] = {'id': 1, 'tag_name': TAG, 'draft': draft, 'prerelease': False,
            'assets': [{'id': i+10, 'name': n, 'state': 'uploaded',
                        'bytes': f'dmg-{TAG}' if n.endswith('.dmg') else n} for i, n in enumerate(names)]}
        self.data['latest'] = '' if draft else TAG

    def run_publisher(self, policy='auto', mode='full', tag=TAG, ok=True):
        self.state.write_text(json.dumps(self.data))
        r = subprocess.run(['/bin/bash', str(ROOT/'scripts/publish-release.sh'), tag, 'cowboyinc/cowchat',
                            str(self.dist), policy, mode], env=self.env, text=True, capture_output=True)
        self.data = json.loads(self.state.read_text())
        self.assertEqual(r.returncode == 0, ok, r.stdout + r.stderr)
        return r

    def mutations(self):
        return [c for c in self.data['calls'] if c[:2] == ['gh', 'release']]

    def test_new_release_verifies_before_publish(self):
        self.run_publisher()
        self.assertEqual(self.data['latest'], TAG)
        self.assertFalse(self.data['release']['draft'])
        self.assertEqual(len(self.data['release']['assets']), 6)
        calls = self.data['calls']
        edit = next(i for i, c in enumerate(calls) if c[:3] == ['gh', 'release', 'edit'])
        self.assertEqual(sum('/releases/assets/' in ' '.join(c) for c in calls[:edit]), 6)
        self.assertEqual([arg for c in calls if c[0] == 'curl' for arg in c if arg.startswith('https://')], [
            f'https://github.com/cowboyinc/cowchat/releases/download/{TAG}/Cowchat.dmg',
            'https://github.com/cowboyinc/cowchat/releases/latest/download/Cowchat.dmg',
        ])

    def test_resume_draft(self):
        self.existing(draft=True, alias=False)
        self.run_publisher()
        self.assertFalse(self.data['release']['draft'])
        self.assertEqual([c[2] for c in self.mutations()], ['upload', 'edit'])

    def test_published_identical_is_read_only(self):
        self.existing()
        self.run_publisher()
        self.assertEqual(self.mutations(), [])

    def test_backfill_without_cli_files(self):
        self.existing(alias=False)
        for n in NAMES[1:]: (self.dist/n).unlink()
        self.run_publisher('preserve', 'installer-only')
        self.assertEqual([c[2] for c in self.mutations()], ['upload'])
        self.assertEqual(self.data['latest'], TAG)
        self.assertTrue(any(c[0] == 'curl' for c in self.data['calls']))

    def test_backfill_rejects_absent_draft_prerelease_and_auto(self):
        for state in ['absent', 'draft', 'prerelease', 'auto']:
            with self.subTest(state=state):
                self.data = {'release': None, 'latest': '', 'calls': []}
                if state != 'absent': self.existing(draft=state == 'draft')
                if state == 'prerelease': self.data['release']['prerelease'] = True
                self.run_publisher('auto' if state == 'auto' else 'preserve', 'installer-only', ok=False)
                self.assertEqual(self.mutations(), [])

    def test_preflight_mismatch_before_upload(self):
        self.existing(alias=False)
        self.data['release']['assets'][3]['bytes'] = 'different'
        self.run_publisher(ok=False)
        self.assertEqual(self.mutations(), [])

    def test_stale_remote_alias_is_never_clobbered(self):
        self.existing()
        next(a for a in self.data['release']['assets'] if a['name'] == 'Cowchat.dmg')['bytes'] = 'dmg-v0.9.0'
        self.run_publisher(ok=False)
        self.assertEqual(self.mutations(), [])

    def test_local_stale_alias_is_replaced_from_versioned_dmg(self):
        (self.dist/'Cowchat.dmg').write_text('dmg-v0.9.0')
        self.run_publisher()
        alias = next(a for a in self.data['release']['assets'] if a['name'] == 'Cowchat.dmg')
        self.assertEqual(alias['bytes'], f'dmg-{TAG}')

    def test_incomplete_asset_stops(self):
        self.existing()
        self.data['release']['assets'][0]['state'] = 'starter'
        self.run_publisher(ok=False)
        self.assertEqual(self.mutations(), [])

    def test_upload_and_verification_failure_leave_draft(self):
        for failure in ['upload_fail', 'corrupt_upload', 'download_fail']:
            with self.subTest(failure=failure):
                self.data = {'release': None, 'latest': '', 'calls': [], failure: True}
                self.run_publisher(ok=False)
                self.assertTrue(self.data['release']['draft'])
                self.assertFalse(any(c[2] == 'edit' for c in self.mutations()))

    def test_upload_conflict_with_identical_bytes_recovers(self):
        self.data['conflict'] = True
        self.run_publisher()
        self.assertEqual(self.data['latest'], TAG)

    def test_latest_ordering_and_preserve(self):
        for latest, policy, expected in [('v0.9.0', 'auto', TAG), ('v0.10.0', 'auto', 'v0.10.0'),
                                         ('v0.9.0', 'preserve', 'v0.9.0')]:
            with self.subTest(latest=latest, policy=policy):
                self.data = {'release': None, 'latest': latest, 'calls': []}
                self.run_publisher(policy)
                self.assertEqual(self.data['latest'], expected)
                if expected != TAG:
                    self.assertIn('--latest=false', next(c for c in self.mutations() if c[2] == 'edit'))

    def test_metadata_failure_never_creates(self):
        self.data['api_fail'] = True
        self.run_publisher(ok=False)
        self.assertEqual(self.mutations(), [])

    def test_malformed_metadata_reports_and_never_creates(self):
        self.data['wrong_shape'] = True
        r = self.run_publisher(ok=False)
        self.assertIn('unexpected release metadata shape', r.stderr)
        self.assertEqual(self.mutations(), [])

    def test_post_publication_lookup_failure_reports_published(self):
        self.data['latest_fail_after_publish'] = True
        r = self.run_publisher(ok=False)
        self.assertFalse(self.data['release']['draft'])
        self.assertIn('release published; public download verification failed', r.stderr)

    def test_bad_latest_and_lookup_failure_never_promote(self):
        for failure in ['malformed', 'latest_fail']:
            self.data = {'release': None, 'latest': 'bad' if failure == 'malformed' else 'v0.9.0', 'calls': []}
            if failure == 'latest_fail': self.data[failure] = True
            self.run_publisher(ok=False)
            self.assertTrue(self.data['release']['draft'])

    def test_bad_tag_and_missing_local_file_fail_before_api(self):
        self.run_publisher(tag='v1.0.0-rc1', ok=False)
        self.assertEqual(self.data['calls'], [])
        (self.dist/NAMES[2]).unlink()
        self.run_publisher(ok=False)
        self.assertEqual(self.data['calls'], [])

    def test_public_failure_is_bounded_and_reports_published(self):
        self.data['probe_fail'] = True
        r = self.run_publisher(ok=False)
        self.assertFalse(self.data['release']['draft'])
        self.assertEqual(len([c for c in self.data['calls'] if c[0] == 'curl']), 5)
        self.assertEqual([c[1] for c in self.data['calls'] if c[0] == 'sleep'], ['2', '4', '8', '16'])
        self.assertIn('release published; public download verification failed', r.stderr)
        probe = next(c for c in self.data['calls'] if c[0] == 'curl')
        self.assertEqual(probe[probe.index('--connect-timeout')+1], '10')
        self.assertEqual(probe[probe.index('--max-time')+1], '300')

    def public_get(self, url, ok=True):
        self.state.write_text(json.dumps(self.data))
        output = self.base/'http-response'
        output.unlink(missing_ok=True)
        r = subprocess.run(['curl', '--fail', url, '-o', str(output)],
                           env=self.env, text=True, capture_output=True)
        self.data = json.loads(self.state.read_text())
        self.assertEqual(r.returncode == 0, ok, r.stderr)
        if not ok:
            self.assertFalse(output.exists())
            return None
        return output.read_text()

    def test_http_reads_remote_bytes_for_tag_and_latest(self):
        self.existing()
        alias = next(a for a in self.data['release']['assets'] if a['name'] == 'Cowchat.dmg')
        alias['bytes'] = 'remote-only-bytes'
        for path in [f'download/{TAG}', 'latest/download']:
            with self.subTest(path=path):
                self.assertEqual(self.public_get(
                    f'https://github.com/cowboyinc/cowchat/releases/{path}/Cowchat.dmg'),
                    'remote-only-bytes')

    def test_http_rejects_unknown_urls_and_unpublished_or_missing_assets(self):
        root = 'https://github.com/cowboyinc/cowchat/releases'
        for url in [f'{root}/download/v9.9.9/Cowchat.dmg',
                    f'{root}/download/{TAG}/missing.dmg',
                    f'{root}/latest/download/missing.dmg',
                    f'{root}/latest',
                    f'{root}/latest/download/Cowchat.dmg?unexpected=1',
                    f'{root.replace("cowboyinc", "other")}/latest/download/Cowchat.dmg',
                    f'{root.replace("github.com", "example.com")}/latest/download/Cowchat.dmg']:
            with self.subTest(url=url):
                self.existing()
                self.public_get(url, ok=False)
        for state in ['draft', 'missing', 'incomplete']:
            with self.subTest(state=state):
                self.existing(draft=state == 'draft', alias=state != 'missing')
                self.data['latest'] = TAG
                if state == 'incomplete':
                    next(a for a in self.data['release']['assets'] if a['name'] == 'Cowchat.dmg')['state'] = 'starter'
                for path in [f'download/{TAG}', 'latest/download']:
                    self.public_get(f'{root}/{path}/Cowchat.dmg', ok=False)

    def test_wrong_byte_response_does_not_corrupt_remote_asset(self):
        self.existing()
        self.data['public_wrong_bytes'] = 'latest'
        root = 'https://github.com/cowboyinc/cowchat/releases'
        self.assertEqual(self.public_get(f'{root}/latest/download/Cowchat.dmg'), 'wrong-public-bytes')
        self.assertEqual(self.public_get(f'{root}/download/{TAG}/Cowchat.dmg'), f'dmg-{TAG}')
        alias = next(a for a in self.data['release']['assets'] if a['name'] == 'Cowchat.dmg')
        self.assertEqual(alias['bytes'], f'dmg-{TAG}')

    def test_public_asset_disappears_after_authenticated_verification(self):
        self.data['public_missing_alias'] = True
        r = self.run_publisher(ok=False)
        self.assertFalse(self.data['release']['draft'])
        self.assertIn('release published; public download verification failed', r.stderr)
        self.assertEqual(len([c for c in self.data['calls'] if c[0] == 'curl']), 5)
        self.assertNotIn('Cowchat.dmg', [a['name'] for a in self.data['release']['assets']])

    def test_public_wrong_bytes_fail_for_tag_and_stable_url(self):
        for path in ['tag', 'latest']:
            with self.subTest(path=path):
                self.data = {'release': None, 'latest': '', 'calls': [], 'public_wrong_bytes': path}
                r = self.run_publisher(ok=False)
                self.assertIn('release published; public download verification failed', r.stderr)
                self.assertFalse(self.data['release']['draft'])
                probes = [c for c in self.data['calls'] if c[0] == 'curl']
                self.assertEqual(len(probes), 5 if path == 'tag' else 6)

    def test_latest_changes_during_probe(self):
        self.data['next_release'] = {
            'id': 2, 'tag_name': 'v1.0.0', 'draft': False, 'prerelease': False,
            'assets': [{'id': 100, 'name': 'Cowchat.dmg', 'state': 'uploaded', 'bytes': 'dmg-v1.0.0'}],
        }
        self.data['change_latest'] = True
        r = self.run_publisher()
        self.assertIn('Latest changed', r.stderr)
        self.assertEqual(self.data['latest'], 'v1.0.0')
        probes = [c for c in self.data['calls'] if c[0] == 'curl']
        # The old release's tag probe succeeds, but five stable-URL responses
        # contain the new release's different bytes before attribution is skipped.
        self.assertEqual(len(probes), 6)

    def test_workflow_wiring(self):
        workflow = (ROOT/'.github/workflows/release.yml').read_text()
        self.assertIn('id: publish', workflow)
        self.assertIn("if: steps.publish.outputs.update_tap == 'true'", workflow)
        self.assertIn('update_tap=false', workflow)
        self.assertIn('group: cowchat-release-publication', workflow)
        self.assertIn('cancel-in-progress: false', workflow)
        self.assertIn('bash scripts/publish-release.sh "$TAG" "$REPO" dist "$PROMOTION_POLICY" full', workflow)
        self.assertIn('inputs.promote_latest', workflow)
        self.assertIn('github.event_name == \'push\'', workflow)
        self.assertIn('needs: [build, mac-app]', workflow)
        self.assertIn('merge-multiple: true', workflow)
        self.assertIn('tar -czf "cowchat-${TAG}-${{ matrix.target }}.tar.gz"', workflow)
        self.assertIn('path: Cowchat-*.dmg', workflow)
        for target in TARGETS: self.assertIn('target: ' + target, workflow)
        self.assertIn('bash scripts/test-publish-release.sh', (ROOT/'.github/workflows/public-build.yml').read_text())


if __name__ == '__main__': unittest.main()
