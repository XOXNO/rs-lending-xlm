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
import branch_coverage_report

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
                source.write_text(
                    f'SF:{path}\nLF:10\nLH:8\n'
                    + ''.join(f'DA:{line},{int(line <= 8)}\n' for line in range(1, 11))
                )
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

    def test_coverage_uses_physical_lines_and_shows_llvm_summary_difference(self):
        source, report = self.base / 'input.lcov', self.base / 'report.md'
        source.write_text(
            'SF:contracts/pool/src/lib.rs\nLF:4\nLH:2\n'
            'DA:1,3\nDA:2,0\nDA:3,1\n'
        )
        hit, total, percent = coverage_report.write_report(source, report, 'merged')
        self.assertEqual((hit, total), (2, 3))
        self.assertAlmostEqual(percent, 200 / 3)
        self.assertIn('| contracts/pool/src/lib.rs | 3 | 2 | 1 | 66.7% | 2 | +1 |', report.read_text())
        self.assertIn('| contracts/pool/src/lib.rs | 2 |', report.read_text())

    def test_branch_report_rejects_absent_instrumentation_and_lists_missed_sites(self):
        source, report = self.base / 'input.json', self.base / 'branches.md'
        branch_file = {
            'filename': str(ROOT / 'contracts/pool/src/ops/withdraw.rs'),
            'summary': {'branches': {'count': 2, 'covered': 1}},
            'branches': [[96, 1, 96, 20, 3, 0, 0, 0, 4]],
        }
        source.write_text(json.dumps({'data': [{'files': [branch_file]}]}))
        self.assertEqual(
            branch_coverage_report.write_report(source, report, 'merged'),
            (1, 2, 50.0),
        )
        self.assertIn(
            '| contracts/pool/src/ops/withdraw.rs | 2 | 1 | 1 | 50.0% | 1 | +0 | 96 |',
            report.read_text(),
        )
        branch_file['branches'].append([96, 1, 96, 20, 0, 4, 0, 0, 4])
        branch_file['summary']['branches'] = {'count': 2, 'covered': 2}
        source.write_text(json.dumps({'data': [{'files': [branch_file]}]}))
        self.assertEqual(
            branch_coverage_report.write_report(source, report, 'merged'),
            (2, 2, 100.0),
        )
        self.assertIn(
            '| contracts/pool/src/ops/withdraw.rs | 2 | 2 | 0 | 100.0% | 0 | +0 | — |',
            report.read_text(),
        )
        branch_file['summary']['branches'] = {'count': 0, 'covered': 0}
        branch_file['branches'] = []
        source.write_text(json.dumps({'data': [{'files': [branch_file]}]}))
        with self.assertRaisesRegex(ValueError, 'zero branches'):
            branch_coverage_report.write_report(source, report, 'merged')

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
    content='SF:contracts/pool/src/lib.rs\\nLF:10\\nLH:8\\n'
    content+=''.join(f'DA:{line},{int(line <= 8)}\\n' for line in range(1, 11))
    pathlib.Path(args[args.index('--output-path')+1]).write_text(content)
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

    def mutation_case(self, statuses=(2, 3, 0), names=None, target='mutants-common', extra='', mode='', config_extra='', empty_diff=False, make_args=()):
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
            '--cargo-arg --config=net.offline=true ' + extra, *make_args]
        result = subprocess.run(command, cwd=case, env=env, capture_output=True, text=True)
        log = case / 'calls.jsonl'
        calls = [json.loads(line) for line in (log.read_text() if log.exists() else '').splitlines()]
        return case, result, [call for call in calls if not call['listing']]

    def test_mutation_retry_identity_arguments_and_pass_evidence(self):
        with patch.dict(os.environ, CARGO_TARGET_DIR=str(self.base / 'unrelated-build')):
            case, result, calls = self.mutation_case((2, 3, 3, 3, 0), extra='-- --nocapture')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([len(c['names']) for c in calls], [4, 1, 1, 1, 1])
        self.assertEqual(calls[1]['names'], calls[0]['names'][:1])
        for call in calls[2:]:
            self.assertEqual(call['names'], calls[1]['names'])
        self.assertEqual([c['github'] for c in calls], ['false'] * 4 + ['true'])
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
        self.assertIn('max_borrow_rate_is_two_ray', calls[3]['args'])
        self.assertIn('test_rate_model_verify_accepts_valid', calls[4]['args'])
        evidence = case / 'reports/mutants.passes/mutants-common'
        for i in range(1, 6):
            self.assertTrue((evidence / f'pass-{i}/mutants.out/log/pass-{i}.log').is_file())
            self.assertTrue((evidence / f'pass-{i}/command.json').is_file())
        first_caught = calls[0]['names'][1]
        self.assertIn(first_caught, (evidence / 'pass-1/mutants.out/caught.txt').read_text())
        for directory in ('mutants.out', 'mutants.out.old'):
            self.assertNotIn(first_caught, (case / 'reports' / directory / 'caught.txt').read_text())

    def test_mutation_failure_propagation_and_empty_scope(self):
        for statuses, mode, success, count in [((1,), '', False, 1), ((4,), '', False, 1),
                ((5,), '', False, 1), ((6,), '', False, 1), ((70,), '', False, 1), ((2, 4), '', False, 2),
                ((2, 2, 2, 2, 2), '', False, 5), ((2, 3, 3, 3, 3), '', False, 5),
                ((0,), '', True, 1),
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

    def test_mutation_pass_replaces_run_wide_workers_and_timeout(self):
        import run_mutants
        head = ['--package', 'controller', '-j', '3', '--jobserver-tasks', '3', '--minimum-test-timeout', '600',
                '--cargo-arg', '-j', '--timeout=9', '-j4']
        self.assertEqual(run_mutants.pass_head(head, ['--test-package', 'controller', '--cargo-test-arg', '-j']), head)
        self.assertEqual(run_mutants.pass_head(head, ['--jobs=1', '--minimum-test-timeout', '1800', '-t', '5']),
                         ['--package', 'controller', '--jobserver-tasks', '3', '--cargo-arg', '-j'])
        self.assertEqual(run_mutants.pass_head(head, ['--jobserver-tasks=2']),
                         ['--package', 'controller', '-j', '3', '--minimum-test-timeout', '600',
                          '--cargo-arg', '-j', '--timeout=9', '-j4'])
        self.assertEqual(run_mutants.pass_head(['--package', 'x', '--test-tool=nextest'], ['--test-tool=cargo']),
                         ['--package', 'x'])

    def test_mutation_controller_retry_passes_use_threads_and_full_suite_cap(self):
        names = [f'contracts/controller/src/strategies/swap.rs:{i}:5: replace candidate_{i} -> bool with false'
                 for i in range(8)]
        lane = ['MUTANTS_RUN_MODE=', 'MUTANTS_TEST_TOOL=nextest', 'MUTANTS_JOBS=3', 'MUTANTS_JOBSERVER_TASKS=3']

        def values(args, flag):
            return [args[i + 1] for i, arg in enumerate(args) if arg == flag]
        _, result, calls = self.mutation_case((2, 0), names=names, target='mutants-controller-core',
                                              make_args=lane)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([len(c['names']) for c in calls], [4, 1])
        core_first, core_full = (call['args'] for call in calls)
        self.assertEqual(values(core_first, '-j'), ['3'])
        self.assertEqual(values(core_full, '--minimum-test-timeout'), ['900'])
        # Strategies start with the focused harness pass, then the full suite.
        _, result, calls = self.mutation_case((2, 0), names=names, target='mutants-controller-strategies',
                                              make_args=lane)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([len(c['names']) for c in calls], [4, 1])

        focused, full = (call['args'] for call in calls)
        tools = lambda args: [arg for arg in args if arg.startswith('--test-tool')]
        self.assertEqual(tools(core_first), ['--test-tool=cargo'])
        self.assertIn('--cargo-test-arg=--tests', core_first)
        self.assertNotIn('--cargo-test-arg=--test-threads=3', core_first)
        self.assertEqual(tools(focused), ['--test-tool=nextest'])
        self.assertEqual(tools(full), ['--test-tool=nextest'])
        for args in (focused, full):
            self.assertEqual(values(args, '-j'), ['1'])
            self.assertEqual(values(args, '--jobserver-tasks'), ['3'])
            self.assertIn('--cargo-test-arg=--test-threads=3', args)
            self.assertFalse(any('--build-jobs' in arg for arg in args))
        self.assertEqual(values(focused, '--minimum-test-timeout'), ['600'])
        self.assertEqual(values(focused, '--shard'), ['0/2'])
        self.assertEqual(values(focused, '--sharding'), ['round-robin'])
        self.assertEqual(values(focused, '--test-package'), ['test-harness'])
        self.assertIn('--cargo-test-arg=--no-tests=pass', focused)
        self.assertTrue(any(arg.startswith('--cargo-test-arg=--filterset=binary_id(=test-harness::strategy)')
                            for arg in focused))
        self.assertEqual(values(full, '--minimum-test-timeout'), ['900'])
        self.assertEqual(values(full, '--test-package'), ['controller', 'governance', 'test-harness'])
        self.assertFalse(any('--filterset' in arg or '--no-tests' in arg for arg in full))
        # Filtersets are nextest syntax: under cargo test the target keeps two passes.
        _, result, calls = self.mutation_case((2, 0), names=names, target='mutants-controller-strategies')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([len(c['names']) for c in calls], [4, 1])
        self.assertEqual(values(calls[1]['args'], '--minimum-test-timeout'), ['900'])
        self.assertFalse(any('--filterset' in arg or '--test-threads' in arg for c in calls for arg in c['args']))

    def test_mutation_focused_harness_binaries_exist(self):
        # nextest rejects a filterset name that is not a workspace test binary,
        # which fails the shard at its baseline. Find the wrong name offline.
        result = subprocess.run(['make', '-n', '-o', '_mutants-check', '-o', '_mutants-harness-prepare',
                                 'mutants-controller-strategies', 'MUTANTS_TEST_TOOL=nextest'],
                                cwd=ROOT, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        expression = re.search(r'--cargo-test-arg=--filterset=(\S+)', result.stdout).group(1).rstrip("'")
        terms = expression.split('|')
        binaries = [re.fullmatch(r'binary_id\(=test-harness::([a-z0-9_]+)\)', term) for term in terms]
        self.assertTrue(terms and all(binaries), expression)
        manifest = (ROOT / 'tests/test-harness/Cargo.toml').read_text()
        self.assertNotRegex(manifest, r'autotests\s*=\s*false|\[\[test\]\]')
        tests = ROOT / 'tests/test-harness/tests'
        for name in (match.group(1) for match in binaries):
            with self.subTest(binary=name):
                self.assertTrue((tests / f'{name}.rs').is_file() or (tests / name / 'main.rs').is_file())

    def test_fuzz_targets_run_in_parallel_and_report_every_crash(self):
        binaries = self.base / 'fuzz-bin'
        binaries.mkdir()
        calls = self.base / 'fuzz-calls.jsonl'
        cargo = binaries / 'cargo'
        cargo.write_text(f'#!{sys.executable}\n' + '''
import json, os, sys, time
args = sys.argv[1:]
target = next(args[i - 1] for i, arg in enumerate(args) if '/corpus/' in arg)
start = time.monotonic()
time.sleep(1.0)
with open(os.environ['FUZZ_CALLS'], 'a') as log:
    log.write(json.dumps({'target': target, 'start': start, 'end': time.monotonic(), 'args': args}) + '\\n')
if target.startswith('crash'):
    print(f'==1== ERROR: libFuzzer: deadly signal in {target}')
    sys.exit(77)
print(f'Done 100 runs in 1 second(s) for {target}')
''')
        cargo.chmod(0o755)
        env = dict(os.environ, PATH=str(binaries) + os.pathsep + os.environ['PATH'], FUZZ_CALLS=str(calls))

        def run(jobs):
            calls.write_text('')
            result = subprocess.run(['make', '-s', '-f', str(ROOT / 'Makefile'), 'fuzz', f'FUZZ_JOBS={jobs}',
                                     'FUZZ_TARGETS=ok_a crash_b ok_c crash_d', f'FUZZ_DIR={self.base / "fuzz"}',
                                     'FUZZ_TIME=7'], cwd=ROOT, env=env, capture_output=True, text=True)
            rows = [json.loads(line) for line in calls.read_text().splitlines()]
            return result, sorted(rows, key=lambda row: row['start'])

        result, rows = run(4)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(sorted(row['target'] for row in rows), ['crash_b', 'crash_d', 'ok_a', 'ok_c'])
        self.assertLess(max(row['start'] for row in rows), min(row['end'] for row in rows), 'targets must overlap')
        for target in ('crash_b', 'crash_d'):
            self.assertIn(f'::error::fuzz {target} crashed (exit 77):', result.stdout)
            self.assertIn(f'deadly signal in {target}', result.stdout)
        for target in ('ok_a', 'ok_c'):
            self.assertIn(f'Done 100 runs in 1 second(s) for {target}', result.stdout)
        self.assertIn('-max_total_time=7', rows[0]['args'])
        self.assertTrue((self.base / 'fuzz/corpus/ok_a').is_dir())

        result, rows = run(1)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([row['target'] for row in rows], ['ok_a', 'crash_b', 'ok_c', 'crash_d'])
        for earlier, later in zip(rows, rows[1:]):
            self.assertGreaterEqual(later['start'], earlier['end'], 'FUZZ_JOBS=1 runs one target at a time')

        result, rows = run(0)
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertEqual(rows, [])

    def test_mutation_state_resumes_only_the_mutants_not_caught_before(self):
        state = self.base / 'mutation-state'
        names = [f'common/src/math/example.rs:{i}:5: replace candidate_{i} -> bool with false' for i in range(8)]
        # Pass 1 misses one mutant, pass 2 times it out, pass 3 catches it.
        _, result, calls = self.mutation_case((2, 3, 0), names=names, make_args=[f'MUTANTS_SAVE_STATE={state}'])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([len(c['names']) for c in calls], [4, 1, 1])
        first_scope = calls[0]['names']
        self.assertEqual((state / 'caught.txt').read_text().splitlines(), sorted(first_scope))

        # One new mutant joins the shard. Only it runs, by exact name, without --shard.
        new = 'common/src/math/example.rs:99:5: replace fresh_candidate -> bool with false'
        resume = [f'MUTANTS_RESUME={state}', f'MUTANTS_SAVE_STATE={state}']
        _, result, calls = self.mutation_case((0,), names=[new] + names, make_args=resume)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([c['names'] for c in calls], [[new]])
        self.assertNotIn('--shard', calls[0]['args'])
        self.assertEqual(calls[0]['config']['examine_re'], ['^' + re.escape(new) + '$'])
        self.assertEqual(set((state / 'caught.txt').read_text().splitlines()), set(first_scope) | {new})

        # Everything in scope is resolved: no mutant runs, and the run passes.
        _, result, calls = self.mutation_case((), names=[new] + names, make_args=resume)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(calls, [])
        self.assertIn('Nothing left to test in this scope.', result.stdout)

    def test_fuzz_rustc_wrapper_drops_coverage_only_outside_the_workspace(self):
        rustc = self.base / 'fake-rustc'
        rustc.write_text(f'#!{sys.executable}\nimport json, os, sys\n'
                         'print(json.dumps([os.environ.get("WRAPPED", ""), *sys.argv[1:]]))\n')
        rustc.chmod(0o755)
        coverage = ['-Cpasses=sancov-module', '-Cllvm-args=-sanitizer-coverage-level=4',
                    '-C', 'llvm-args=-sanitizer-coverage-inline-8bit-counters', '-C', 'passes=sancov-module']
        kept = ['-Zsanitizer=address', '-Cdebug-assertions', '-C', 'overflow-checks', '--cfg', 'fuzzing']

        def wrap(source, env=None):
            result = subprocess.run(['bash', str(ROOT / 'scripts/fuzz_rustc_wrapper.sh'), str(rustc),
                                     '--crate-name', 'x', source, *coverage, *kept],
                                    env=dict(os.environ, **(env or {})), capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads(result.stdout)

        registry = '/home/u/.cargo/registry/src/index.crates.io-1/soroban-env-host-1/src/lib.rs'
        self.assertEqual(wrap(registry)[1:], ['--crate-name', 'x', registry, *kept])
        workspace = str(ROOT / 'contracts/pool/src/lib.rs')
        self.assertEqual(wrap(workspace)[1:], ['--crate-name', 'x', workspace, *coverage, *kept])
        chained = self.base / 'next-wrapper'
        chained.write_text('#!/usr/bin/env bash\nWRAPPED=next exec "$@"\n')
        chained.chmod(0o755)
        self.assertEqual(wrap(registry, {'FUZZ_NEXT_RUSTC_WRAPPER': str(chained)})[0], 'next')

    def test_mutation_lib_only_passes_test_packages_without_other_test_targets(self):
        # `--lib` builds and runs only the library's unit tests. On a package with
        # integration-test targets it would skip those tests without a sign.
        import shlex
        import tomllib
        lib_only = set()
        for manifest in ROOT.glob('*/**/Cargo.toml'):
            if 'target' in manifest.parts or 'vendor' in manifest.parts:
                continue
            data = tomllib.loads(manifest.read_text())
            package = data.get('package', {})
            if package.get('autotests') is False and not any(data.get(k) for k in ('test', 'bench', 'example')):
                lib_only.add(package['name'])
        self.assertTrue({'common', 'controller', 'pool', 'governance'} <= lib_only)
        targets = re.findall(r'^(mutants-[a-z0-9-]+):', (ROOT / 'Makefile').read_text(), re.M)
        checked = 0
        for target in sorted(set(targets) - {'mutants-diff'}):
            result = subprocess.run(['make', '-n', '-o', '_mutants-check', '-o', '_mutants-harness-prepare', target,
                                     'MUTANTS_TEST_TOOL=nextest'], cwd=ROOT, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            for line in result.stdout.splitlines():
                if 'run_mutants.py' not in line:
                    continue
                words = shlex.split(line.replace('\\', ' '))
                split = words.index('--')
                head, runner = words[split + 1:], words[:split]
                stages = [shlex.split(w.split('=', 1)[1]) for w in runner if w.startswith('--pass=')] or [[]]
                for stage in stages:
                    args = stage + head
                    if '--cargo-arg=--lib' not in args:
                        continue
                    source = stage if '--test-package' in stage else head
                    packages = {source[i + 1] for i, w in enumerate(source) if w == '--test-package'}
                    with self.subTest(target=target, stage=stage):
                        self.assertTrue(packages)
                        self.assertLessEqual(packages, lib_only)
                    checked += 1
        self.assertGreater(checked, 5)

    def test_fuzz_wasm_builds_every_module_the_fuzz_and_proptest_code_loads(self):
        loaded = set()
        for root in ('tests/fuzz/fuzz_targets', 'tests/fuzz/src', 'tests/test-harness/src',
                     'tests/test-harness/tests/fuzz'):
            for path in (ROOT / root).rglob('*.rs'):
                loaded |= set(re.findall(r'\b([a-z0-9_]+)\.wasm\b', path.read_text()))
        packages = re.search(r'^FUZZ_WASM_PACKAGES := (.+)$', (ROOT / 'Makefile').read_text(), re.M).group(1).split()
        self.assertTrue(loaded)
        self.assertEqual({name.replace('_', '-') for name in loaded}, set(packages))
        for package in packages:
            self.assertTrue(list(ROOT.glob(f'*/{package}/Cargo.toml')), package)

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
