"""Run mutation passes with exact retries and retained evidence (Python 3.11+)."""

import argparse
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tomllib

# The only statement is cfg(certora); source-aware tests guard this assumption.
NATIVE_NOOP = r'^contracts/controller/src/spec_hooks\.rs:[0-9]+:[0-9]+: replace solvency_gate_checked with \(\)$'
# Forwarded Cargo values can themselves look like mutation options.
CARGO_VALUE_OPTIONS = ('--cargo-arg', '-C', '--cargo-test-arg')
INCLUDE_OPTIONS = ('--re', '-F', '--regex', '--examine-regex', '--examine-re')


def option_values(args, *flags):
    values, i = [], 0
    while i < len(args):
        arg = args[i]
        if (arg in flags or arg in CARGO_VALUE_OPTIONS) and i + 1 == len(args):
            raise ValueError(f'{arg} requires a value')
        for flag in flags:
            if arg == flag:
                values.append(args[i + 1])
            elif arg.startswith(flag + '='):
                values.append(arg[len(flag) + 1:])
            elif len(flag) == 2 and arg.startswith(flag) and len(arg) > 2:
                values.append(arg[2:])
        i += 2 if arg in flags or arg in CARGO_VALUE_OPTIONS else 1
    return values


def without(args, valued=(), switches=()):
    result, i = [], 0
    while i < len(args):
        arg = args[i]
        if arg in valued:
            i += 2
            continue
        if arg in CARGO_VALUE_OPTIONS:
            result.extend(args[i:i + 2])
            i += 2
            continue
        if arg not in switches and not any(arg.startswith(f + '=') or
                (len(f) == 2 and arg.startswith(f) and len(arg) > 2) for f in valued):
            result.append(arg)
        i += 1
    return result

def write_config(path, config):
    path.write_text(''.join(f'{key} = {str(value).lower() if isinstance(value, float) else json.dumps(value, ensure_ascii=False)}\n' for key, value in config.items()))


def run(label, passes, args, allow_empty=False):
    args = list(args)
    split = 0
    while split < len(args) and args[split] != '--':
        grouped = re.fullmatch(r'-([vV]+)([CLjtFefEDpdo])(.*)', args[split])
        if grouped:
            args[split:split + 1] = ['-' + grouped[1], '-' + grouped[2] + grouped[3]]
            split += 1
        split += 2 if args[split] in CARGO_VALUE_OPTIONS else 1
    head, tail = args[:split], args[split:]
    switches = without(head, CARGO_VALUE_OPTIONS)
    directory = option_values(head, '--dir', '-d') or ['.']
    locate = [os.environ.get('CARGO', 'cargo'), 'locate-project', '--workspace', '--message-format=plain']
    manifest = option_values(head, '--manifest-path')
    if manifest:
        locate += ['--manifest-path', str(Path(manifest[-1]).resolve())]
    workspace = Path(subprocess.check_output(locate, cwd=directory[-1], text=True).strip()).parent
    selected_config = option_values(head, '--config')
    if len(selected_config) > 1 or switches.count('--no-config') > 1:
        raise ValueError('configuration options cannot be repeated')
    if selected_config and '--no-config' in switches:
        raise ValueError('--config and --no-config conflict')
    config_path = Path(selected_config[-1]) if selected_config else workspace / '.cargo/mutants.toml'
    config = {} if '--no-config' in switches or not config_path.exists() else tomllib.loads(config_path.read_text())
    if selected_config and not config_path.exists():
        raise ValueError(f'mutation config missing: {config_path}')
    features = (list(config.get('features', [])) + option_values(head, '--features', '--cargo-arg', '-C', '--cargo-test-arg')
        + config.get('additional_cargo_args', []) + config.get('additional_cargo_test_args', []) + tail
        + [value for key, value in os.environ.items() if key.endswith('RUSTFLAGS')])
    if '--no-config' not in switches and not (config.get('all_features') or '--all-features' in switches or
            any('certora' in feature or '--all-features' in feature for feature in features)):
        config['exclude_re'] = config.get('exclude_re', []) + [NATIVE_NOOP]
    output = option_values(head, '--output', '-o') or [os.environ.get('CARGO_MUTANTS_OUTPUT', config.get('output'))]
    output = Path(output[-1]).resolve() if output[-1] is not None else workspace
    evidence = output / 'mutants.passes' / label
    if evidence.exists():
        shutil.rmtree(evidence)
    evidence.mkdir(parents=True)
    head = without(head, ('--config',), ('--no-config',))
    base_config = evidence / 'base.toml'
    write_config(base_config, config)
    stages = [shlex.split(stage) for stage in passes] or [[]]
    listing = ['cargo', 'mutants'] + without(head, switches=('--list', '--list-files', '--json', '--diff'))
    listing += stages[0] + ['--config', str(base_config), '--list', '--json'] + tail
    result = subprocess.run(listing, text=True, capture_output=True)
    sys.stderr.write(result.stderr)
    if result.returncode:
        sys.stderr.write(result.stdout)
        return result.returncode
    owned = {row['name'] for row in json.loads(result.stdout.strip() or '[]')}
    (evidence / 'scope.json').write_text(json.dumps(sorted(owned), indent=2) + '\n')
    print(f'Mutation scope: {len(owned)} mutants ({len(stages)} pass(es))', flush=True)
    if not owned:
        print('No mutants matched scope/shard.', flush=True)
        return 0 if allow_empty else 1
    live = output / 'mutants.out'
    for number, tests in enumerate(stages, 1):
        expected = owned if number == 1 else pending
        pass_dir = evidence / f'pass-{number}'
        pass_dir.mkdir()
        retry_config = base_config
        if number > 1:
            head = without(head, ('--shard', '--sharding') + INCLUDE_OPTIONS)
            if '--iterate' not in without(head, CARGO_VALUE_OPTIONS):
                head.append('--iterate')
            config['examine_re'] = ['^' + re.escape(name) + '$' for name in sorted(pending)]
            retry_config = pass_dir / 'retry.toml'
            write_config(retry_config, config)
        command = ['cargo', 'mutants'] + head + tests + ['--config', str(retry_config)] + tail
        env = os.environ.copy()
        if number < len(stages):
            env['GITHUB_ACTIONS'] = 'false'
        previous = live.stat().st_ino if live.exists() else None
        status = subprocess.run(command, env=env).returncode
        fresh = live.exists() and live.stat().st_ino != previous
        if fresh:
            shutil.copytree(live, pass_dir / 'mutants.out')
        (pass_dir / 'command.json').write_text(json.dumps({'argv': command, 'exit': status,
            'fresh_output': fresh}, indent=2) + '\n')
        if status not in (0, 2, 3) or '--list' in switches or '--list-files' in switches:
            return status
        if not fresh:
            raise ValueError('cargo-mutants produced no fresh output')
        selected = [row['name'] for row in json.loads((live / 'mutants.json').read_text())]
        if set(selected) != expected or len(selected) != len(expected):
            raise ValueError('cargo-mutants selection changed from the expected scope')
        outcome = json.loads((live / 'outcomes.json').read_text())
        results = [row for row in outcome['outcomes'] if isinstance(row['scenario'], dict)]
        completed = [row['scenario']['Mutant']['name'] for row in results]
        if (set(completed) != expected or len(completed) != len(expected) or
                outcome['total_mutants'] != len(expected) or not outcome['end_time'] or
                any(not row['phase_results'] or row['summary'] not in
                    ('Success', 'CaughtMutant', 'MissedMutant', 'Unviable', 'Timeout') for row in results)):
            raise ValueError('cargo-mutants produced incomplete outcome evidence')
        pending = {line for name in ('missed.txt', 'timeout.txt')
                   for line in (live / name).read_text().splitlines() if line}
        if not pending <= expected:
            raise ValueError('cargo-mutants reported mutants outside the owned scope')
        if pending != {row['scenario']['Mutant']['name'] for row in results
                       if row['summary'] in ('MissedMutant', 'Timeout')}:
            raise ValueError('cargo-mutants pending files disagree with outcomes')
        reported_status = 3 if any(row['summary'] == 'Timeout' for row in results) else 2 if pending else 0
        if status != reported_status:
            raise ValueError('cargo-mutants exit status disagrees with outcomes')
        if not pending:
            return 0
        if number == len(stages):
            return status or 1
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--label', required=True)
    parser.add_argument('--pass', dest='passes', action='append', default=[])
    parser.add_argument('--allow-empty', action='store_true')
    parser.add_argument('args', nargs=argparse.REMAINDER)
    options = parser.parse_args()
    if not re.fullmatch(r'[A-Za-z0-9_-]+', options.label):
        parser.error('label must be a Make target name')
    args = options.args[1:] if options.args[:1] == ['--'] else options.args
    try:
        status = run(options.label, options.passes, args, options.allow_empty)
        return status if status >= 0 else 128 - status
    except (ValueError, KeyError, TypeError, OSError, subprocess.CalledProcessError) as error:
        print(f'mutation runner: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
