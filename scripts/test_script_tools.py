"""Offline negative controls for reports and Make failure propagation."""
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import coverage_report

ROOT = Path(__file__).resolve().parents[1]


class ScriptTools(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        (self.base / 'scripts').symlink_to(ROOT / 'scripts', target_is_directory=True)

    def test_coverage_paths_and_counts(self):
        source, report = self.base / 'input.lcov', self.base / 'report.md'
        with patch.object(coverage_report, 'ROOT', Path('/renamed/checkout')):
            for path in ('/renamed/checkout/contracts/pool/src/lib.rs', '/cached/checkout/common/src/lib.rs',
                         'contracts/pool/src/lib.rs'):
                source.write_text(f'SF:{path}\nLF:10\nLH:8\n')
                self.assertEqual(coverage_report.write_report(source, report, 'merged'), (8, 10, 80.0))
        for path in ('/home/.cargo/registry/hyper/src/common/lock.rs',
                     '/work/checkout/mock/mock-foo/src/common/helper.rs',
                     '/vendor/something/contracts/pool/src/lib.rs',
                     'vendor/contracts/pool/src/lib.rs', '../common/src/lib.rs', 'common/tests/lib.rs'):
            self.assertFalse(coverage_report.keep(path, 'merged'), path)
        for data in ('', 'SF:/outside/lib.rs\nLF:10\nLH:8\n',
                     'SF:contracts/pool/src/lib.rs\nLF:0\nLH:0\n',
                     'SF:contracts/pool/src/lib.rs\nLF:1\nLH:2\n',
                     'SF:contracts/pool/src/lib.rs\nLF:1\n'):
            source.write_text(data)
            with self.assertRaises(ValueError):
                coverage_report.write_report(source, report, 'merged')

    def test_scout_incomplete_reports_fail(self):
        valid = self.base / 'pool.json'
        valid.write_text(json.dumps({'summary': {'by_severity': {}}, 'findings': []}))
        command = [sys.executable, str(ROOT / 'scripts/scout-summary.py')]
        self.assertEqual(subprocess.run(command + [str(valid)], capture_output=True).returncode, 0)
        for data in ('{', '{}', '{"summary":{"by_severity":{}},"findings":null}',
                     '{"summary":{"by_severity":{}},"findings":[{"vulnerability_id":"real"}]}',
                     '{"summary":{"by_severity":{"high":1}},"findings":[]}'):
            (self.base / 'controller.json').write_text(data)
            result = subprocess.run(command + [str(self.base)], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn('No findings', result.stdout)
        for args in ([], [str(self.base / 'missing.json')]):
            self.assertNotEqual(subprocess.run(command + args, capture_output=True).returncode, 0)

    def test_make_propagates_test_and_report_failures(self):
        binaries = self.base / 'bin'
        binaries.mkdir()
        cargo = binaries / 'cargo'
        cargo.write_text(f'#!{sys.executable}\n' + '''
import os, pathlib, sys
args=sys.argv[1:]; mode=os.environ.get('REVIEW_FAILURE')
if args[:2]==['llvm-cov','report']:
    if mode=='report': sys.exit(43)
    pathlib.Path(args[args.index('--output-path')+1]).write_text('SF:contracts/pool/src/lib.rs\\nLF:10\\nLH:8\\n')
sys.exit(42 if mode=='harness' and 'test-harness' in args else 0)
''')
        cargo.chmod(0o755)
        env = dict(os.environ, PATH=str(binaries) + os.pathsep + os.environ['PATH'])
        for failure in ('', 'harness', 'report'):
            result = subprocess.run(['make', '-f', str(ROOT / 'Makefile'), 'coverage-merged',
                                     f'COV_DIR={self.base / "coverage"}', f'TEST_HARNESS_DIR={self.base / "harness"}'],
                                    cwd=self.base, env=env | {'REVIEW_FAILURE': failure}, capture_output=True, text=True)
            self.assertEqual(result.returncode == 0, failure == '', result.stdout + result.stderr)
            self.assertNotIn('(ignored)', result.stderr)

    def mutation_case(self, statuses=(2, 3, 0), names=None, target='mutants-common', extra='', mode='', config_extra='', empty_diff=False):
        case = Path(tempfile.mkdtemp(dir=self.base))
        (case / 'scripts').symlink_to(ROOT / 'scripts', target_is_directory=True)
        (case / '.cargo').mkdir()
        (case / '.cargo/mutants.toml').write_text('gitignore=true\nexclude_re=["never"]\n'
            'examine_re=["^common/"]\nadditional_cargo_args=["--offline"]\noutput="config-output"\n' + config_extra)
        (case / 'Cargo.toml').write_text('[workspace]\n')
        (case / 'pr.diff').write_text('' if empty_diff else 'diff controlled by fake cargo\n')
        binaries = case / 'bin'
        binaries.mkdir()
        cargo = binaries / 'cargo'
        cargo.write_text(f'#!{sys.executable}\n' + '''
import json, math, os, pathlib, re, shutil, sys, tomllib
raw_args=sys.argv[1:]; args=[]; i=0
while i<len(raw_args) and raw_args[i]!='--':
    if raw_args[i] in ('--cargo-arg','-C','--cargo-test-arg'): i+=2; continue
    args.append(raw_args[i]); i+=1
root=pathlib.Path(os.environ['MUTATION_CASE'])
def values(*flags):
    result=[]
    for i,a in enumerate(args):
        for flag in flags:
            if a==flag: result.append(args[i+1])
            elif a.startswith(flag+'='): result.append(a[len(flag)+1:])
    return result
if args[0]=='locate-project': print(root/'Cargo.toml'); sys.exit(0)
if '--version' in args: print('cargo-mutants 27.1.0'); sys.exit(0)
config=tomllib.loads(pathlib.Path(values('--config')[-1]).read_text())
rows=json.loads(pathlib.Path(os.environ['MUTATION_NAMES']).read_text())
include=config.get('examine_re',[])+values('--re','-F','--regex','--examine-regex','--examine-re')
exclude=config.get('exclude_re',[])+values('--exclude-re','-E')
include=[re.compile(p) for p in include]; exclude=[re.compile(p) for p in exclude]
rows=[n for n in rows if (not include or any(p.search(n) for p in include)) and not any(p.search(n) for p in exclude)]
output=pathlib.Path((values('--output','-o') or [os.environ.get('CARGO_MUTANTS_OUTPUT',config.get('output',root))])[-1])
live=output/'mutants.out'; old=output/'mutants.out.old'
if '--iterate' in args:
    caught={line for d in (live,old) for f in ('caught.txt','unviable.txt') if (d/f).exists() for line in (d/f).read_text().splitlines()}
    rows=[n for n in rows if n not in caught]
if values('--shard'):
    k,count=map(int,values('--shard')[-1].split('/')); width=math.ceil(len(rows)/count); rows=rows[k*width:(k+1)*width]
listing='--list' in args
with (root/'calls.jsonl').open('a') as f:
    f.write(json.dumps({'args':raw_args,'names':rows,'listing':listing,'config':config,'github':os.environ.get('GITHUB_ACTIONS'),
      'pool_wasm':os.environ.get('POOL_WASM_PATH'),'cases':os.environ.get('PROPTEST_CASES'),'seed':os.environ.get('PROPTEST_RNG_SEED')})+'\\n')
if listing:
    if os.environ['MUTATION_MODE']=='list-failure': sys.exit(42)
    print(json.dumps([{'name':n} for n in rows])); sys.exit(0)
counter=root/'count'; index=int(counter.read_text()) if counter.exists() else 0; counter.write_text(str(index+1))
status=json.loads(os.environ['MUTATION_STATUSES'])[index]
if os.environ['MUTATION_MODE']=='no-artifacts': sys.exit(status)
if old.exists(): shutil.rmtree(old)
if live.exists(): live.rename(old)
live.mkdir(parents=True)
for f in ('caught.txt','missed.txt','timeout.txt','unviable.txt'): (live/f).write_text('')
if os.environ['MUTATION_MODE']=='zero-work': rows=[]
selected=rows+['foreign mutant'] if os.environ['MUTATION_MODE']=='foreign-selected' else rows
(live/'mutants.json').write_text(json.dumps([{'name':n} for n in selected]))
if status in (2,3):
    pending=rows if os.environ['MUTATION_MODE']=='all-missed' else rows[:1]
    if os.environ['MUTATION_MODE']=='foreign': pending=['foreign mutant']
    (live/('missed.txt' if status==2 else 'timeout.txt')).write_text(''.join(n+'\\n' for n in pending))
    caught=[n for n in rows if n not in pending]
else: caught=rows if status==0 else []
(live/'caught.txt').write_text(''.join(n+'\\n' for n in caught))
outcomes=[{'scenario':{'Mutant':{'name':n}},'summary':'CaughtMutant' if n in caught else 'MissedMutant' if status==2 else 'Timeout',
           'phase_results':[{'phase':'Test'}]} for n in rows]
if os.environ['MUTATION_MODE']=='partial-outcomes': outcomes=outcomes[:-1]
if os.environ['MUTATION_MODE']=='empty-phases':
    for row in outcomes: row['phase_results']=[]
if os.environ['MUTATION_MODE'] in ('pending-disagrees','status-disagrees'):
    (live/'missed.txt').write_text(rows[0]+'\\n')
if os.environ['MUTATION_MODE']=='status-disagrees': outcomes[0]['summary']='MissedMutant'
if os.environ['MUTATION_MODE']=='unknown-summary': outcomes[0]['summary']='Failure'
if os.environ['MUTATION_MODE']!='missing-outcomes':
    (live/'outcomes.json').write_text(json.dumps({'outcomes':outcomes,'total_mutants':len(rows),
      'end_time':None if os.environ['MUTATION_MODE']=='unfinished-outcomes' else 'finished'}))
(live/'log').mkdir(); (live/'log'/f'pass-{index+1}.log').write_text('retained control log')
sys.exit(status)
''')
        cargo.chmod(0o755)
        if names is None:
            names = [f'common/src/math/example.rs:{i}:5: replace candidate_{i} -> bool with false' for i in range(8)]
        (case / 'names.json').write_text(json.dumps(names))
        env = dict(os.environ, PATH=str(binaries) + os.pathsep + os.environ['PATH'],
            MUTATION_CASE=str(case), MUTATION_NAMES=str(case / 'names.json'), MUTATION_STATUSES=json.dumps(statuses),
            MUTATION_MODE=mode, GITHUB_ACTIONS='true', CARGO_TARGET_DIR='target')
        command = ['make', '-f', str(ROOT / 'Makefile'), '-o', '_mutants-check', '-o', '_mutants-harness-prepare',
            target, 'MUTANTS_RUN_MODE=--in-place', 'MUTANTS_SHARD=0/2', 'MUTANTS_FILTER=--re . --exclude-re never',
            'MUTANTS_EXTRA_ARGS=--output reports --baseline skip --profile dev --cargo-test-arg=--exact '
            '--cargo-arg --config=net.offline=true ' + extra]
        result = subprocess.run(command, cwd=case, env=env, capture_output=True, text=True)
        log = case / 'calls.jsonl'
        calls = [json.loads(line) for line in (log.read_text() if log.exists() else '').splitlines()]
        return case, result, [call for call in calls if not call['listing']]

    def test_mutation_retry_identity_arguments_and_pass_evidence(self):
        with patch.dict(os.environ, CARGO_TARGET_DIR=str(self.base / 'unrelated-build')):
            case, result, calls = self.mutation_case(extra='-- --nocapture')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([len(c['names']) for c in calls], [4, 1, 1])
        self.assertEqual(calls[1]['names'], calls[0]['names'][:1])
        self.assertEqual(calls[2]['names'], calls[1]['names'])
        self.assertEqual([c['github'] for c in calls], ['false', 'false', 'true'])
        for i, call in enumerate(calls):
            self.assertEqual('--shard' in call['args'], i == 0)
            self.assertEqual('--re' in call['args'], i == 0)
            for arg in ('--in-place', '--baseline', 'skip', '--profile', 'dev', '--cargo-test-arg=--exact', '--cargo-arg', '--config=net.offline=true', '--', '--nocapture'):
                self.assertIn(arg, call['args'])
            self.assertEqual(call['config']['gitignore'], True)
            self.assertEqual(call['config']['additional_cargo_args'], ['--offline'])
            self.assertEqual(call['config']['output'], 'config-output')
            self.assertEqual((call['cases'], call['seed']), ('1', '0'))
            self.assertEqual(Path(call['pool_wasm']).resolve(), (case / 'target/wasm32v1-none/release/pool.wasm').resolve())
        self.assertNotIn('test-harness', calls[1]['args'])
        self.assertIn('test-harness', calls[2]['args'])
        evidence = case / 'reports/mutants.passes/mutants-common'
        for i in range(1, 4):
            self.assertTrue((evidence / f'pass-{i}/mutants.out/log/pass-{i}.log').is_file())
            self.assertTrue((evidence / f'pass-{i}/command.json').is_file())
        first_caught = calls[0]['names'][1]
        self.assertIn(first_caught, (evidence / 'pass-1/mutants.out/caught.txt').read_text())
        for directory in ('mutants.out', 'mutants.out.old'):
            self.assertNotIn(first_caught, (case / 'reports' / directory / 'caught.txt').read_text())

    def test_mutation_failure_propagation_and_empty_scope(self):
        for statuses, mode, success, count in [((1,), '', False, 1), ((4,), '', False, 1),
                ((5,), '', False, 1), ((6,), '', False, 1), ((70,), '', False, 1), ((2, 4), '', False, 2),
                ((2, 2, 2), '', False, 3), ((2, 3, 3), '', False, 3), ((0,), '', True, 1),
                ((0,), 'no-artifacts', False, 1), ((2,), 'foreign', False, 1), ((0,), 'list-failure', False, 0)]:
            with self.subTest(statuses=statuses, mode=mode):
                _, result, calls = self.mutation_case(statuses, mode=mode)
                self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
                self.assertEqual(len(calls), count)
                for fault, message in [('no-artifacts', 'no fresh output'), ('foreign', 'outside the owned scope'),
                                       ('list-failure', 'Error 42')]:
                    if mode == fault:
                        self.assertIn(message, result.stderr)
        for target, success in [('mutants-common', False), ('mutants-diff', True)]:
            _, result, calls = self.mutation_case(names=[], target=target)
            self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
            self.assertEqual(calls, [])
        _, result, calls = self.mutation_case(target='mutants-diff', empty_diff=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(calls, [], 'an empty diff must stop before listing or mutation')
        _, result, calls = self.mutation_case((2,), target='mutants-diff')
        self.assertNotEqual(result.returncode, 0, 'a missed PR mutant must fail Make')
        self.assertEqual(len(calls), 1)

    def test_mutation_cli_aliases_opaque_values_and_complete_work(self):
        import run_mutants
        for flag in ('--cargo-arg', '-C', '--cargo-test-arg', '--config', '--output', '--dir'):
            with self.subTest(missing=flag), self.assertRaisesRegex(ValueError, 'requires a value'):
                run_mutants.option_values([flag], '--config', '--output', '--dir')
        for flag in ('--re', '-F', '--regex', '--examine-regex', '--examine-re'):
            for extra in (f'{flag} .', f'{flag}=.'):
                with self.subTest(extra=extra):
                    _, result, calls = self.mutation_case(extra=extra + ' --sharding slice')
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertEqual([len(c['names']) for c in calls], [4, 1, 1])
                    for call in calls[1:]:
                        self.assertFalse(any(a == flag or a.startswith(flag + '=') for a in call['args']))
                        self.assertNotIn('--sharding', call['args'])
        for flag in ('--cargo-arg', '-C', '--cargo-test-arg'):
            with self.subTest(opaque=flag):
                _, result, calls = self.mutation_case(extra=flag + ' -- -- --nocapture')
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual([len(c['names']) for c in calls], [4, 1, 1])
                for call in calls:
                    index = max(i for i, arg in enumerate(call['args']) if arg == flag)
                    self.assertEqual(call['args'][index + 1], '--')
                    self.assertEqual(call['args'][-2:], ['--', '--nocapture'])
        for mode in ('zero-work', 'foreign-selected', 'missing-outcomes', 'partial-outcomes',
                     'unfinished-outcomes', 'empty-phases', 'pending-disagrees', 'status-disagrees', 'unknown-summary'):
            with self.subTest(mode=mode):
                case, result, calls = self.mutation_case((0,), mode=mode)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(len(calls), 1)
                self.assertTrue((case / 'reports/mutants.passes/mutants-common/pass-1/command.json').is_file())

    def test_mutation_retry_config_avoids_argument_length_limit(self):
        names = [f'common/src/example.rs:{i}:5: replace ' + ('long_name_' * 25) + str(i) + ' with ()' for i in range(1600)]
        case, result, calls = self.mutation_case((2, 0), names=names, mode='all-missed')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(calls[1]['names'], calls[0]['names'])
        self.assertGreater((case / 'reports/mutants.passes/mutants-common/pass-2/retry.toml').stat().st_size, 131072)
        self.assertLess(max(map(len, calls[1]['args'])), 4096)

    def test_mutation_noop_exclusion_is_source_scoped_and_formal_aware(self):
        import run_mutants
        import tomllib
        config = {'exclude_re': ['quoted \" value\\ \U0001f642'], 'gitignore': True,
                  'features': ['testing'], 'timeout_multiplier': float('inf')}
        generated = self.base / 'roundtrip.toml'
        run_mutants.write_config(generated, config)
        self.assertEqual(tomllib.loads(generated.read_text()), config)
        manifest = tomllib.loads((ROOT / 'contracts/controller/Cargo.toml').read_text())
        self.assertEqual(manifest['features'].get('default', []), [])
        source = (ROOT / 'contracts/controller/src/spec_hooks.rs').read_text()
        body = re.search(r'fn solvency_gate_checked\([^)]*\)\s*\{([^}]*)\}', source).group(1)
        self.assertEqual(body.strip(), '#[cfg(feature = "certora")]\n    health_ghost::record_gate(_account);')
        name = 'contracts/controller/src/spec_hooks.rs:16:5: replace solvency_gate_checked with ()'
        self.assertRegex(name, run_mutants.NATIVE_NOOP)
        self.assertNotRegex(name.replace('spec_hooks.rs', 'other.rs'), run_mutants.NATIVE_NOOP)
        compiler_flags = {key: '' for key in os.environ if key.endswith('RUSTFLAGS')}
        with patch.dict(os.environ, compiler_flags):
            for extra, config_extra, success in [('', '', False), ('--features certora', '', True),
                    ('--all-features', '', True), ('--no-config', '', True), ('--cargo-arg=--all-features', '', True),
                    ('', 'features=["certora"]\n', True), ('', 'all_features=true\n', True)]:
                _, result, calls = self.mutation_case((0,), names=[name], target='mutants-controller-core',
                    extra=extra, config_extra=config_extra)
                self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
                self.assertEqual(len(calls), int(success))
        with patch.dict(os.environ, {'CARGO_TARGET_TEST_RUSTFLAGS': '--cfg feature="certora"'}):
            _, result, calls = self.mutation_case((0,), names=[name], target='mutants-controller-core')
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(len(calls), 1)

    def test_proptest_one_rejects_missing_matches_and_propagates_failures(self):
        binaries = self.base / 'bin'
        binaries.mkdir()
        calls = self.base / 'cargo.jsonl'
        cargo = binaries / 'cargo'
        cargo.write_text(f'#!{sys.executable}\n' + '''
import json, os, sys
args = sys.argv[1:]
phase = 'list' if '--list' in args else 'run'
with open(os.environ['REVIEW_CARGO_LOG'], 'a') as stream:
    stream.write(json.dumps({'phase': phase, 'args': args,
                             'cases': os.environ.get('PROPTEST_CASES')}) + '\\n')
assert args[:6] == ['test', '--release', '-p', 'test-harness', '--test', 'fuzz']
assert args[7:] == ['--', '--list' if phase == 'list' else '--test-threads=1']
if phase == 'list':
    print('prop_selected: test' if args[6] == 'prop_selected' else '0 tests, 0 benchmarks')
    sys.exit(37 if os.environ['REVIEW_FAILURE'] == 'list' else 0)
sys.exit(43 if os.environ['REVIEW_FAILURE'] == 'run' else 0)
''')
        cargo.chmod(0o755)
        env = dict(os.environ, PATH=str(binaries) + os.pathsep + os.environ['PATH'],
                   REVIEW_CARGO_LOG=str(calls))
        for selected, failure, phases, error in [
                ('prop_selected', '', ['list', 'run'], ''),
                ('missing', '', ['list'], 'matched no tests'),
                ('prop_selected', 'list', ['list'], 'Error 37'),
                ('prop_selected', 'run', ['list', 'run'], 'Error 43'),
                ('   ', '', [], 'requires TEST'),
                (None, '', [], 'requires TEST')]:
            with self.subTest(selected=selected, failure=failure):
                calls.unlink(missing_ok=True)
                command = ['make', '-f', str(ROOT / 'Makefile'), 'proptest-one', 'PROPTEST_CASES=17']
                if selected is not None:
                    command.append(f'TEST={selected}')
                result = subprocess.run(command, cwd=self.base, env=env | {'REVIEW_FAILURE': failure},
                                        capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 2 if error else 0, result.stdout + result.stderr)
                if error:
                    self.assertIn(error, result.stdout + result.stderr)
                observed = [json.loads(line) for line in calls.read_text().splitlines()] if calls.exists() else []
                self.assertEqual([row['phase'] for row in observed], phases)
                for row in observed:
                    self.assertEqual(row['args'][6], selected)
                    self.assertEqual(row['cases'], '17')

    def test_deploy_rejects_missing_or_unstrippable_artifacts(self):
        source, deploy = self.base / 'source', self.base / 'deploy'
        source.mkdir()
        deploy.mkdir()
        (source / 'controller.wasm').write_bytes(b'not reached')
        for invalid in (False, True):
            if invalid:
                (source / 'pool.wasm').write_bytes(b'invalid WASM')
            (deploy / 'pool.wasm').write_bytes(b'stale')
            result = subprocess.run(['make', '-f', str(ROOT / 'Makefile'), '-o', 'optimize', 'deploy-artifacts',
                                     'WASM_SIZE_CONTRACTS=pool controller', f'OPTIMIZED_DIR={source}',
                                     f'DEPLOY_DIR={deploy}', 'MAKE=true'], cwd=self.base, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((deploy / 'pool.wasm').exists())
            self.assertFalse((deploy / 'controller.wasm').exists())


if __name__ == '__main__':
    unittest.main()
