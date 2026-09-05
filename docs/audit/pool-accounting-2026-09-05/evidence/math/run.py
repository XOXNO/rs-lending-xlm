"""Run from any location: python3 run.py /path/to/rs-lending-xlm (offline)."""
import json
import os
from pathlib import Path
import subprocess
import sys

here = Path(__file__).resolve().parent
repo = Path(sys.argv[1] if len(sys.argv) > 1 else "/Users/mihaieremia/GitHub/rs-lending-xlm").resolve()
target = Path(os.environ.get("CARGO_TARGET_DIR", repo / "target")).resolve()
env = dict(os.environ, RUSTC_WRAPPER="", CARGO_TARGET_DIR=str(target))
commands = []

def run(args, log):
    commands.append({"cwd": str(repo), "argv": list(map(str, args)), "log": log})
    (here / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
    with (here / log).open("w") as stream:
        subprocess.run(args, cwd=repo, env=env, stdout=stream, stderr=subprocess.STDOUT, check=True)

run(["cargo", "test", "-p", "common", "math::", "--offline", "--locked"], "common-math-tests.log")
run([sys.executable, here / "generate.py"], "generate.log")

sdk_libs = []
for fingerprint in (target / "debug/.fingerprint").glob("soroban-sdk-*/lib-soroban_sdk.json"):
    if "testutils" in json.loads(fingerprint.read_text()).get("features", ""):
        digest = fingerprint.parent.name.removeprefix("soroban-sdk-")
        lib = target / f"debug/deps/libsoroban_sdk-{digest}.rlib"
        if lib.exists():
            sdk_libs.append(lib)
sdk_lib = max(sdk_libs, key=lambda path: path.stat().st_mtime)
source = (here / "probe.rs").read_text().replace("/Users/mihaieremia/GitHub/rs-lending-xlm", str(repo))
(here / "compiled_probe.rs").write_text(source)
run(["rustc", "--edition=2021", "-C", "overflow-checks=yes", "-C", "debug-assertions=no",
     here / "compiled_probe.rs", "--extern", f"soroban_sdk={sdk_lib}",
     "-L", f"dependency={target / 'debug/deps'}", "-o", here / "probe"], "compile.log")
run([here / "probe"], "probe.log")
print((here / "probe.log").read_text())
