#!/usr/bin/env python3
"""Dry-run publication guard: injected failures and artifact swaps block it."""
from pathlib import Path
from copy import deepcopy
import json, hashlib, subprocess, sys, re
from controlled import MANIFEST
from controlled import collect as collect_controlled
import tempfile, textwrap
from release_gate import LANES, verify
import release_gate
from artifacts import CONTRACTS, DISTRIBUTION_FILES, distribution

candidate = {'source_sha': 'candidate', 'artifacts': {'controller.wasm': 'a'*64}}
proof = {'status': 'pass', 'lanes': {l:1 for l in LANES}, 'candidate': candidate, 'controlled': {'candidate':candidate,'status':'pass','manifest_sha256':hashlib.sha256(MANIFEST.read_bytes()).hexdigest(),'log_sha256':'c'*64,'cases':json.loads(MANIFEST.read_text())}}
packaged = {'files': {'candidate.json': 'd'*64}, 'manifest_sha256': 'e'*64}
proof['distribution'] = packaged
verify(candidate, proof, packaged)
for mutation in ('failure', 'partial', 'empty', 'different_sha', 'different_hash'):
    broken = deepcopy(proof)
    if mutation == 'failure': broken['status'] = 'fail'
    if mutation == 'partial': del broken['lanes']['sdk']
    if mutation == 'empty': broken['lanes']['sdk'] = 0
    if mutation == 'different_sha': broken['candidate']['source_sha'] = 'other'
    if mutation == 'different_hash': broken['candidate']['artifacts']['controller.wasm'] = 'b'*64
    try:
        verify(candidate, broken, packaged)
    except (ValueError,AssertionError,KeyError):
        continue
    raise AssertionError(f'publication incorrectly allowed: {mutation}')
print('Release guard dry run passed: failures and artifact substitution block publication')

# The CLI must bind every uploaded byte, not only the production code. Fake
# module bytes suffice here: this regression tests packaging, not execution.
with tempfile.TemporaryDirectory() as directory:
    root=Path(directory); dist=root/'dist'; dist.mkdir()
    for name in DISTRIBUTION_FILES:
        (dist/name).write_bytes(b'original '+name.encode())
    # Valid module containing only a custom docs section. Both versions have
    # identical executable code; exact SDK bytes still belong to the release.
    (dist/'sdk-controller.wasm').write_bytes(b'\0asm\1\0\0\0\0\6\4docsA')
    bundled={'source_sha':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
        'artifacts':{f'{c}.wasm':hashlib.sha256((dist/f'{c}.wasm').read_bytes()).hexdigest() for c in CONTRACTS}}
    (dist/'candidate.json').write_text(json.dumps(bundled))
    (dist/'sdk-manifest.json').write_text(json.dumps({'sdk': 'metadata'}))
    log=dist/'controlled-tests.log'
    log.write_text('\n'.join('test '+case['test']+' ... ok' for case in json.loads(MANIFEST.read_text()))+'\n')
    controlled=collect_controlled(log,bundled)
    packaged=distribution(dist,create=True)
    original_validate=release_gate.validate
    release_gate.validate=lambda run,expected_lane: 1
    try:
        for lane in LANES:
            run=root/f'run-{lane}'; run.mkdir()
            (run/'candidate.json').write_text(json.dumps(bundled))
            (run/'controlled.json').write_text(json.dumps(controlled))
            (run/'controlled-tests.log').write_bytes(log.read_bytes())
        proof=release_gate.collect(root,'run',dist)
        assert proof['distribution']==packaged
        local=release_gate.collect(root,'run')
        assert 'distribution' not in local
        try: verify(bundled,local,packaged)
        except ValueError: pass
        else: raise AssertionError('standalone proof authorized publication without distribution')
    finally:
        release_gate.validate=original_validate
    proof_path=root/'proof.json'; proof_path.write_text(json.dumps(proof))
    def publication(succeeds):
        result=subprocess.run([sys.executable,str(Path(__file__).with_name('release_gate.py')),
            'verify',str(dist),str(proof_path)],capture_output=True,text=True)
        assert (result.returncode==0)==succeeds, result.stdout+result.stderr
    publication(True)
    sdk=dist/'sdk-controller.wasm'; original_sdk=sdk.read_bytes()
    sdk.write_bytes(original_sdk[:-1]+b'B')
    publication(False)
    sdk.write_bytes(original_sdk)
    # SDK metadata-only changes must fail even when executable code is intact.
    for name in ['sdk-controller.wasm','sdk-mock_reflector.wasm','sdk-manifest.json',
                 'controller.wasm.sha256','candidate.json','distribution.json']:
        path=dist/name; original=path.read_bytes()
        path.write_bytes(original+b'\n')
        publication(False)
        path.write_bytes(original)
    for name in ['sdk-controller.wasm','sdk-manifest.json','controller.wasm.sha256','distribution.json']:
        path=dist/name; original=path.read_bytes(); path.unlink()
        publication(False)
        path.write_bytes(original)
    for name in ['unexpected.wasm','unexpected.wasm.sha256']:
        path=dist/name; path.write_bytes(b'unbound')
        publication(False)
        try: distribution(dist,create=True)
        except AssertionError: pass
        else: raise AssertionError('unexpected upload asset entered distribution manifest')
        path.unlink()
    # Rehashing a substituted SDK file cannot replace the E2E-bound manifest.
    (dist/'sdk-controller.wasm').write_bytes(b'substituted')
    distribution(dist,create=True)
    publication(False)
print('Complete distribution substitution checks passed')

# Selected time tests passing must not hide a different failing workspace test.
with tempfile.TemporaryDirectory() as directory:
    log=Path(directory)/'controlled.log'
    passed='\n'.join('test '+case['test']+' ... ok' for case in json.loads(MANIFEST.read_text()))
    for failure in ['test result: FAILED. 1 passed; 1 failed;', 'error: could not compile `other-contract`']:
        log.write_text(passed+'\n'+failure+'\n')
        try: collect_controlled(log,candidate)
        except AssertionError: pass
        else: raise AssertionError('workspace failure hidden by selected controlled cases')

# Keep the actual workflow publication behind both job success and exact-byte
# verification; dry-run dispatch must never enter the GitHub release mutation.
workflow = (Path(__file__).resolve().parents[2]/'.github/workflows/release.yml').read_text()
# Queued jobs consume the build checkout's resolved SHA even if its branch or
# explicit tag moves. Resolve the actual workflow expressions against that move.
build = workflow.split('  testnet-e2e:',1)[0]
assert 'id: checkout' in build
# The attested build reads no restorable cache and builds from the committed lockfile.
BUILD_ACTIONS = {'actions/checkout', 'dtolnay/rust-toolchain', 'actions/setup-node',
    'actions/attest-build-provenance', 'actions/upload-artifact'}
def build_job_problems(text):
    job = text.split('  testnet-e2e:',1)[0].split('\n  build:\n',1)[1]
    problems = []
    for body in re.split(r'^      - ', job, flags=re.M)[1:]:
        action = re.search(r'^\s*uses:\s*([^@\s]+)@', body, re.M)
        if action and action.group(1) not in BUILD_ACTIONS:
            problems.append(f'build job uses {action.group(1)}')
        if re.search(r'^\s+cache[\w-]*:', body, re.M):
            problems.append(f'build step sets a cache input: {body.splitlines()[0]}')
    if not re.search(r'^\s+run: make candidate-wasm\b', job, re.M):
        problems.append('build job does not run make candidate-wasm')
    if 'stellar contract build' in text:
        problems.append('release.yml runs stellar contract build directly')
    return problems
assert build_job_problems(workflow) == [], build_job_problems(workflow)
node_step = "          node-version: '24'\n"
cli_step = '      - name: Install stellar-cli\n'
for mutated in [
    workflow.replace(cli_step, "      - uses: actions/cache@" + '0'*40 + "\n        with:\n          path: target\n          key: k\n" + cli_step, 1),
    workflow.replace(node_step, node_step + "          cache: npm\n", 1),
    workflow.replace('run: make candidate-wasm ', 'run: make ', 1),
    workflow.replace('      - name: Package release WASM\n', "      - run: stellar contract build --locked --package pool\n      - name: Package release WASM\n", 1),
]:
    assert mutated != workflow and build_job_problems(mutated), 'build-job gate accepted a mutated workflow'
builder = (Path(__file__).resolve().parents[2]/'scripts/build_e2e_wasm.sh').read_text()
contract_builds = [line for line in builder.splitlines() if 'stellar contract build' in line]
assert contract_builds and all('--locked' in line for line in contract_builds), contract_builds
output = re.search(r'      source_sha: \$\{\{ (.*?) \}\}', build).group(1)
refs = re.findall(r'          ref: \$\{\{ (.*?) \}\}', workflow)
assert len(refs) == 3
assert refs[0] == 'inputs.tag || github.sha'
assert output == 'steps.checkout.outputs.commit'
assert refs[1:] == ['needs.build.outputs.source_sha'] * 2
def resolve(expression, context):
    return next(context[part.strip()] for part in expression.split('||') if context[part.strip()])
for tag in ['', 'v1.2.3']:
    event_sha, built_sha, advanced_sha = 'a'*40, ('b'*40 if tag else 'a'*40), 'c'*40
    context = {'inputs.tag':tag, 'github.sha':event_sha, 'github.ref':'refs/heads/main',
        'steps.checkout.outputs.commit':built_sha}
    assert resolve(refs[0],context) == (tag or event_sha)
    context['needs.build.outputs.source_sha'] = resolve(output,context)
    moved_refs = {'refs/heads/main':advanced_sha, 'v1.2.3':advanced_sha}
    for expression in refs[1:]:
        ref = resolve(expression,context)
        assert moved_refs.get(ref,ref) == built_sha, 'queued job followed a moved ref'
publish = workflow.split('  publish:',1)[1]
assert 'needs: [build, testnet-e2e]' in publish
assert publish.index('release_gate.py verify') < publish.index('gh release upload')
assert "if: github.event_name != 'workflow_dispatch' || !inputs.dry_run" in publish
assert "if: github.event_name == 'workflow_dispatch' && inputs.dry_run && inputs.inject_e2e_failure" in workflow
assert workflow.index('Inject failed E2E gate') < workflow.index('Run parallel testnet e2e')
# Only a guaranteed pre-deployment failure may use the hosted runner. Normal
# dry runs and tag releases retain the live runner and all release lanes.
e2e = workflow.split('  testnet-e2e:',1)[1].split('  publish:',1)[0]
runner = re.search(r'    runs-on: \$\{\{ (.*?) \}\}', e2e).group(1)
guard = re.search(r"      - name: Inject failed E2E gate.*?        if: (.*?)\n", e2e, re.S).group(1)
assert runner == guard + " && 'ubuntu-latest' || 'self-hosted'"
injection = e2e.split('      - name: Inject failed E2E gate',1)[1].split('      - name:',1)[0]
assert re.search(r'^          exit 1$', injection, re.M)
assert 'continue-on-error:' not in injection
assert 'continue-on-error:' not in e2e
assert 'if:' not in e2e.split('      - name: Run parallel testnet e2e',1)[1].split('      - name:',1)[0]

def step(section, name):
    return section.split(f'      - name: {name}\n',1)[1].split('      - name:',1)[0]
def run_step(body, gh_out, env=None):
    script = textwrap.dedent(body.split('        run: |\n',1)[1])
    with tempfile.TemporaryDirectory() as directory:
        gh = Path(directory)/'gh'
        gh.write_text('#!/usr/bin/env bash\necho "$*" >> "$GH_LOG"\n'
            'case "$1 $2" in "api "*|"release view") [ -n "$GH_OUT" ] || exit 1; echo "$GH_OUT";; esac\n')
        gh.chmod(0o755)
        log = Path(directory)/'calls'; log.touch()
        result = subprocess.run(['bash','-ec',script], capture_output=True, env={
            'PATH':f'{directory}:/usr/bin:/bin', 'GH_LOG':str(log), 'GH_OUT':gh_out,
            'GITHUB_REPOSITORY':'o/r', 'SOURCE_SHA':'a'*40, 'RELEASE_TAG':'v1.2.3', **(env or {})})
        return result.returncode, log.read_text()
on_main = step(build, 'Release commit is on main')
assert "if: ${{ !(github.event_name == 'workflow_dispatch' && inputs.dry_run) }}" in on_main
assert build.index('Release commit is on main') < build.index('Build canonical candidate')
for status, allowed in [('identical',True),('behind',True),('ahead',False),('diverged',False),('',False)]:
    code, calls = run_step(on_main, status)
    assert (code == 0) == allowed, f'release commit status {status!r}'
    assert calls.startswith(f'api repos/o/r/compare/main...{"a"*40}')
publish_job = publish.split('    steps:',1)[0]
release_tag = re.search(r'RELEASE_TAG: \$\{\{ (.*?) \}\}', publish).group(1)
assert f'group: release-publish-${{{{ {release_tag} }}}}' in publish_job
assert 'cancel-in-progress: false' in publish_job
release = step(publish, 'Publish release')
for draft, allowed, created in [('',True,True),('true',True,False),('false',False,False)]:
    code, calls = run_step(release, draft)
    assert (code == 0) == allowed, f'existing release draft={draft!r}'
    assert ('release create' in calls) == created
    assert ('release upload' in calls) == allowed
print('Release commit and published-release guards passed')
assert workflow.index('sdk_manifest.py dist') < workflow.index('artifacts.py distribution-create dist') < workflow.index('name: Upload artifact')
assert 'collect tests/integration/runs "$RUN_TS" artifacts/wasm/deploy' in workflow
assert workflow.count('dist/distribution.json') == 3  # Attestation, upload, publication.
for name in ['e2e.yml','release.yml']:
    content=(Path(__file__).resolve().parents[2]/'.github/workflows'/name).read_text()
    cargo_step=content.split('cargo test --workspace --no-fail-fast 2>&1 | tee controlled-tests.log',1)[0].rsplit('run: |',1)[1]
    assert 'set -o pipefail' in cargo_step, f'{name}: tee masks failed workspace tests'
    build,live=content.split('  e2e:' if name=='e2e.yml' else '  testnet-e2e:',1)
    live=live.split('  publish:',1)[0]
    assert build.count('integration-fixtures')==1
    assert 'name: e2e-fixtures' in build and 'name: e2e-fixtures' in live
    assert 'artifacts/wasm/fixtures/*.wasm\n            artifacts/wasm/fixtures/SHA256SUMS' in build
    assert 'path: artifacts/wasm/fixtures' in live
    assert not any(tool in live for tool in ['rust-toolchain@','actions/cache@','integration-fixtures','cargo '])
    assert live.index('sha256sum --strict --check SHA256SUMS') < live.index('bash tests/integration/scenarios/parallel_e2e.sh')
    # Exercise the actual workflow checksum commands: substitutions and missing
    # fixture files must fail before any lane sends transactions.
    checksum=re.search(r'        run: (sha256sum .* > SHA256SUMS)',build).group(1)
    check=re.search(r'cd artifacts/wasm/fixtures && (sha256sum .*?)\)',live).group(1)
    with tempfile.TemporaryDirectory() as directory:
        fixture=Path(directory)/'fixture.wasm'; fixture.write_bytes(b'fixture')
        subprocess.run(['bash','-ec',checksum],cwd=directory,check=True)
        def checked():
            return subprocess.run(['bash','-ec',check],cwd=directory,capture_output=True).returncode==0
        assert checked()
        fixture.write_bytes(b'changed'); assert not checked()
        fixture.unlink(); assert not checked()
assert 'e2e-fixtures' not in publish and 'artifacts/wasm/fixtures' not in publish
print('Build-only fixture handoff and checksum rejection checks passed')

EXPECTED_LANES = {
    'wallets': ['agg-admin', 'agg-core', 'agg-gov', 'blend', 'flash-a', 'flash-b', 'liq-a', 'liq-b', 'liq-c', 'prod-caller', 'prod-full', 'sdk', 'stress'],
    'deploy_protocol': ['agg-admin', 'agg-core', 'agg-gov', 'blend', 'flash-a', 'flash-b', 'liq-a', 'liq-b', 'liq-c', 'prod-caller', 'prod-full', 'sdk', 'stress'],
    'flow_real_markets': ['agg-admin', 'agg-core', 'blend', 'sdk'],
    'flow_fund_usdc': ['agg-admin', 'agg-core', 'agg-gov', 'sdk'],
    'flow_seed_liquidity': ['agg-admin', 'agg-core', 'flash-a', 'flash-b', 'sdk'],
    'flow_lifecycle': ['agg-core'],
    'flow_flash_loans': ['agg-admin'],
    'flow_strategies': ['agg-core'],
    'flow_admin': ['agg-admin'],
    'flow_gap_hunt_admin': ['agg-admin'],
    'flow_pool_surface': ['agg-admin'],
    'flow_swap_aggregator_admin': ['agg-gov'],
    'flow_governance': ['agg-gov'],
    'flow_admin_upgrade': ['agg-core'],
    'flow_teardown': ['agg-admin', 'agg-core', 'agg-gov', 'blend', 'flash-a', 'flash-b', 'liq-a', 'liq-b', 'liq-c', 'prod-caller', 'prod-full', 'sdk', 'stress'],
    'flow_liq_setup': ['liq-a', 'liq-b', 'liq-c'],
    'flow_liq_single': ['liq-a'],
    'flow_liq_bulk': ['liq-b'],
    'flow_liq_spoke': ['liq-a'],
    'flow_liq_credit': ['liq-a'],
    'flow_liq_credit_rejections': ['liq-a'],
    'flow_clean_bad_debt': ['liq-b'],
    'flow_force_socialize_and_recap': ['liq-b'],
    'flow_spoke_flags_and_curve': ['liq-a'],
    'flow_liq_deprecated_spoke_credit': ['liq-c'],
    'flow_defindex_strategy': ['liq-c'],
    'flow_stress_setup': ['stress'],
    'flow_stress_supply_frontier': ['stress'],
    'flow_stress_borrow_frontier:single': ['stress'],
    'flow_stress_dualify': ['stress'],
    'flow_stress_borrow_frontier:dual': ['stress'],
    'flow_stress_liq_frontier': ['stress'],
    'flow_xoxno_oracle': ['liq-c'],
    'flow_flash_position_markets': ['flash-a', 'flash-b'],
    'flow_flash_position_fund': ['flash-a', 'flash-b'],
    'flow_flash_position': ['flash-a', 'flash-b'],
    'flow_flash_position_matrix': ['flash-a'],
    'flow_flash_position_gaps': ['flash-b'],
    'flow_flash_position_gates': ['flash-b'],
    'flow_flash_position_malicious': ['flash-a'],
    'flow_blend_hub_liquidity': ['blend'],
    'flow_blend_allowlist': ['blend', 'sdk'],
    'flow_blend_rejects': ['blend'],
    'flow_blend_migrate': ['blend'],
    'flow_sdk_lifecycle': ['sdk'],
    'flow_sdk_strategy': ['sdk'],
    'flow_sdk_blend': ['sdk'],
    'flow_production_fixtures': ['prod-caller', 'prod-full'],
    'flow_production_operator': ['prod-caller', 'prod-full'],
    'flow_production_caller': ['prod-caller'],
    'flow_production_upgrade': ['prod-full'],
    'flow_same_market': ['agg-admin'],
    'flow_nft': ['agg-admin'],
    'flow_risk_refresh': ['agg-gov'],
    'flow_stress_composed': ['stress'],
    'flow_stress_delayed': ['stress'],
    'flow_liq_multi_hub': ['liq-b'],
    'flow_blend_multireserve': ['blend'],
    'flow_production_lending': ['prod-caller'],
    'flow_sdk_errors': ['sdk'],
}
ROOT = Path(__file__).resolve().parents[2]
manifest = json.loads((ROOT/'tests/integration/cases.json').read_text())
assert len(manifest) == len(EXPECTED_LANES) and all(EXPECTED_LANES.values())
assert {c['id']: sorted(set(c['lanes']) & LANES) for c in manifest} == EXPECTED_LANES
for case in manifest:
    if case['id'] in {'wallets', 'deploy_protocol', 'flow_teardown'}:
        assert LANES <= set(case['lanes']), case['id']
orchestrator = (ROOT/'tests/integration/scenarios/parallel_e2e.sh').read_text()
release_lanes = re.findall(r"^RELEASE_LANES='([^']*)'$", orchestrator, re.M)
assert len(release_lanes) == 1, release_lanes
release_lanes = release_lanes[0].split()
assert len(release_lanes) == len(set(release_lanes)) and set(release_lanes) == LANES, release_lanes
dispatch = (ROOT/'.github/workflows/e2e.yml').read_text()
choices = re.search(r'^        options:\n((?:          - .*\n)+)', dispatch, re.M).group(1)
choices = re.findall(r'^          - (\S+)$', choices, re.M)
assert len(choices) == len(set(choices)) and set(choices) == LANES | {'all'}, choices
everything = re.findall(r"inputs\.lanes == 'all' && '([^']*)'", dispatch)
assert len(everything) == 1, everything
everything = everything[0].split()
assert len(everything) == len(set(everything)) and set(everything) == LANES, everything
script_for = re.search(r'^script_for\(\) \{\n.*?^\}\n', orchestrator, re.M | re.S).group(0)
routes = {'strategies.sh': {'strategies'}}
for lane in sorted(LANES):
    script = subprocess.run(['bash', '-c', script_for + 'script_for "$1"', '_', lane],
                            capture_output=True, text=True, check=True).stdout.strip()
    routes.setdefault(script, set()).add(lane)
scenarios = ROOT/'tests/integration/scenarios'
assert {p.name for p in scenarios.glob('*.sh') if re.search(r'^\s*run_case\s', p.read_text(), re.M)} == set(routes), routes
selected = {c['id']: set(c['lanes']) for c in manifest}
for script, lanes in routes.items():
    ids = re.findall(r'^\s*run_case\s+(\S+)', (scenarios/script).read_text(), re.M)
    assert len(ids) == len(set(ids)), (script, ids)
    for case_id in ids:
        assert selected.get(case_id, set()) & lanes, f'{script} runs {case_id}, which no lane routed to it selects'
    for lane in lanes:
        assert {i for i, l in selected.items() if lane in l} <= set(ids), (script, lane)
print('Lane membership, orchestrator, dispatch and scenario run_case pins passed')

production = (scenarios/'production.sh').read_text()
arms = re.search(r'^case "\$E2E_LANE" in\n.*?^esac\n', production, re.M | re.S).group(0)
only = {}
for lane in routes['production.sh']:
    only[lane] = subprocess.run(['bash', '-c', f'E2E_LANE="$1"\n{arms}printf %s "$PROD_CONFIG_ONLY"', '_', lane],
                                capture_output=True, text=True, check=True).stdout
full_lanes = {lane for lane, value in only.items() if value == ''}
assert full_lanes == {'prod-full'} and set(only) - full_lanes == {'prod-caller'}, only
pinned = {'upgrade': 0, 'replay': 0}
for case in manifest:
    labels = {a['label'] for a in case['required_actions']}
    if any(re.fullmatch(r'operator_upgrade\w+Hash', label) for label in labels):
        pinned['upgrade'] += 1
        assert set(case['lanes']) & LANES <= full_lanes, case['id']
        assert {'prod_upgrade_full_config', 'prod_policy_equal'} <= labels, case['id']
    if 'operator_setupAll_replay' in labels:
        pinned['replay'] += 1
        assert set(case['lanes']) & full_lanes, case['id']
assert all(pinned.values()), pinned
print('Upgrade proofs and the setup replay run only on the full mainnet-shaped production config')
