"""Non-network review probes. Every mutation is confined to newly created temp repos."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
OX = ROOT / 'target' / 'debug' / ('ox.exe' if os.name == 'nt' else 'ox')
BASE = Path(tempfile.mkdtemp(prefix='oxidize-review-'))
ENV = {**{k: v for k, v in os.environ.items() if not k.startswith('GIT_')},
       'GIT_CONFIG_NOSYSTEM': '1', 'GIT_CONFIG_GLOBAL': os.devnull,
       'GIT_AUTHOR_NAME': 'Review', 'GIT_AUTHOR_EMAIL': 'review@example.invalid',
       'GIT_COMMITTER_NAME': 'Review', 'GIT_COMMITTER_EMAIL': 'review@example.invalid'}
results = []


def run(repo, program, *args):
    p = subprocess.run([str(program), *args], cwd=repo, env=ENV,
                       capture_output=True, text=True, timeout=30)
    return {'command': [Path(program).name, *args], 'code': p.returncode,
            'stdout': p.stdout.strip(), 'stderr': p.stderr.strip()}


def git(repo, *args):
    result = run(repo, 'git', *args)
    assert result['code'] == 0, result
    return result['stdout']


def repo(name):
    path = BASE / name
    path.mkdir()
    git(path, 'init', '-b', 'main')
    git(path, 'config', 'core.autocrlf', 'false')
    (path / 'a.txt').write_text('base\n')
    git(path, 'add', '.')
    git(path, 'commit', '-m', 'base')
    return path


p = repo('checkout')
git(p, 'checkout', '-b', 'other')
(p / 'a.txt').write_text('other\n')
git(p, 'commit', '-am', 'other')
git(p, 'checkout', 'main')
(p / 'a.txt').write_text('precious uncommitted edit\n')
oracle = run(p, 'git', 'checkout', 'other')
r = run(p, OX, 'checkout', 'other')
results.append({'case': 'dirty checkout', 'git': oracle, 'ox': r,
                'content_after_ox': (p / 'a.txt').read_text()})

p = repo('index-v4')
git(p, 'update-index', '--index-version=4')
(p / 'b.txt').write_text('second\n')
r = run(p, OX, 'add', 'b.txt')
results.append({'case': 'v4 mutation', 'ox': r, 'git': run(p, 'git', 'ls-files', '--stage')})

p = repo('index-lock')
lock = p / '.git/index.lock'
lock.write_text('existing owner\n')
r = run(p, OX, 'add', 'a.txt')
results.append({'case': 'existing index lock', 'ox': r, 'lock_exists_after': lock.exists()})

p = repo('ref-lock')
lock = p / '.git/refs/heads/main.lock'
lock.write_text('existing owner\n')
r = run(p, OX, 'update-ref', 'refs/heads/main', git(p, 'rev-parse', 'HEAD'))
results.append({'case': 'existing ref lock', 'ox': r, 'lock_exists_after': lock.exists()})

p = repo('packed')
git(p, 'gc', '--prune=now')
results.append({'case': 'packed checkout', 'ox': run(p, OX, 'checkout', 'main'),
                'git': run(p, 'git', 'checkout', 'main')})

p = repo('rm-dirty')
(p / 'a.txt').write_text('precious uncommitted edit\n')
oracle = run(p, 'git', 'rm', 'a.txt')
r = run(p, OX, 'rm', 'a.txt')
results.append({'case': 'rm without force', 'git': oracle, 'ox': r,
                'file_exists_after': (p / 'a.txt').exists()})

p = repo('rm-untracked')
(p / 'folder').mkdir()
(p / 'folder/tracked.txt').write_text('tracked')
git(p, 'add', '.')
git(p, 'commit', '-m', 'folder')
(p / 'folder/precious.txt').write_text('untracked work')
r = run(p, OX, 'rm', '-r', 'folder')
results.append({'case': 'recursive rm untracked collateral', 'ox': r,
                'untracked_exists_after': (p / 'folder/precious.txt').exists()})

p = repo('ref-traversal')
r = run(p, OX, 'branch', '../../review-sentinel')
results.append({'case': 'ref namespace traversal', 'ox': r,
                'wrote_git_root_sentinel': (p / '.git/review-sentinel').exists()})

p = repo('conflict-tree')
git(p, 'checkout', '-b', 'other')
(p / 'a.txt').write_text('other\n')
git(p, 'commit', '-am', 'other')
git(p, 'checkout', 'main')
(p / 'a.txt').write_text('main\n')
git(p, 'commit', '-am', 'main')
run(p, 'git', 'merge', 'other')
results.append({'case': 'write-tree with unresolved conflict',
                'git': run(p, 'git', 'write-tree'), 'ox': run(p, OX, 'write-tree')})

p = repo('stash-stack')
for value in ['one\n', 'two\n']:
    (p / 'a.txt').write_text(value)
    git(p, 'stash', 'push', '-m', value.strip())
r = run(p, OX, 'stash', 'pop')
results.append({'case': 'stash pop with two entries', 'ox': r,
                'git_stash_list_after': run(p, 'git', 'stash', 'list'),
                'reflog_still_exists': (p / '.git/logs/refs/stash').exists()})

p = repo('tree-traversal')
blob = git(p, 'rev-parse', 'HEAD:a.txt')
raw_tree = b'100644 ../escaped-tree-sentinel\0' + bytes.fromhex(blob)
tree = subprocess.run(['git', 'hash-object', '--literally', '-w', '-t', 'tree', '--stdin'],
                      cwd=p, env=ENV, input=raw_tree, capture_output=True, check=True).stdout.decode().strip()
commit = git(p, 'commit-tree', tree, '-m', 'malformed tree probe')
git(p, 'update-ref', 'refs/heads/malformed', commit)
r = run(p, OX, 'checkout', 'malformed')
results.append({'case': 'checkout tree path escapes repository', 'ox': r,
                'wrote_outside_repo_inside_temp_root': (BASE / 'escaped-tree-sentinel').exists()})

p = repo('delete-last')
git(p, 'rm', 'a.txt')
results.append({'case': 'commit deletion of last tracked file',
                'ox': run(p, OX, 'commit', '-m', 'delete last file')})

p = repo('mode-only')
git(p, 'update-index', '--chmod=+x', 'a.txt')
results.append({'case': 'staged executable bit change', 'git': run(p, 'git', 'diff', '--cached', '--summary'),
                'ox': run(p, OX, 'status')})

p = repo('local-push-source')
destination = BASE / 'local-push-destination'
git(p, 'clone', str(p), str(destination))
git(p, 'remote', 'add', 'origin', str(destination))
(destination / 'remote.txt').write_text('remote-only commit\n')
git(destination, 'add', '.')
git(destination, 'commit', '-m', 'remote-only')
remote_before = git(destination, 'rev-parse', 'HEAD')
(p / 'local.txt').write_text('local-only commit\n')
git(p, 'add', '.')
git(p, 'commit', '-m', 'local-only')
orphan = subprocess.run(['git', 'hash-object', '-w', '--stdin'], cwd=p, env=ENV,
                        input=b'unreachable review-only blob', capture_output=True, check=True).stdout.decode().strip()
r = run(p, OX, 'push', 'origin', 'main')
results.append({'case': 'local push overwrites divergent checked-out branch and copies unreachable blob',
                'ox': r, 'remote_before': remote_before,
                'remote_after': git(destination, 'rev-parse', 'HEAD'),
                'remote_worktree_still_has_old_file': (destination / 'remote.txt').exists(),
                'unreachable_blob_present_at_destination': run(destination, 'git', 'cat-file', '-e', orphan)['code'] == 0})

print(json.dumps({'temporary_root': str(BASE), 'results': results}, indent=2))
