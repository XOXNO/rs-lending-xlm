#!/usr/bin/env python3
"""Strip doc strings and informational meta from a deploy WASM.

Rustdoc comments on contract entrypoints and types embed verbatim into the
on-chain spec and count against the network's contractMaxSizeBytes. Deploy
artifacts do not need them (reference docs live in the interface crates and
the published documentation), so this rewrites every `doc` field in the spec
to an empty string via the stellar CLI's XDR codec and reassembles the WASM.

contractmetav0 sections (name/version strings from contractmeta! and the
toolchain) and error-enum spec entries (code→name maps for client bindgen)
are likewise informational only — the host requires just contractenvmetav0
— so they are dropped entirely. Error definitions live in common/src and
the interface crates.

Usage: strip_spec_docs.py <in.wasm> <out.wasm>
"""

import json
import subprocess
import sys
import tempfile

SPEC_SECTION = b"contractspecv0"
META_SECTION = b"contractmetav0"


def read_leb128(data: bytes, i: int) -> tuple[int, int]:
    value = shift = 0
    while True:
        byte = data[i]
        i += 1
        value |= (byte & 0x7F) << shift
        if not (byte & 0x80):
            return value, i
        shift += 7


def write_leb128(value: int) -> bytes:
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def blank_docs(node):
    if isinstance(node, dict):
        return {
            key: ("" if key == "doc" and isinstance(val, str) else blank_docs(val))
            for key, val in node.items()
        }
    if isinstance(node, list):
        return [blank_docs(item) for item in node]
    return node


def main() -> None:
    src, dst = sys.argv[1], sys.argv[2]
    wasm = open(src, "rb").read()

    sections = []  # (id, header_offset, body, name)
    i = 8
    while i < len(wasm):
        start = i
        sec_id = wasm[i]
        i += 1
        size, i = read_leb128(wasm, i)
        body = wasm[i : i + size]
        i += size
        name = None
        if sec_id == 0:
            nlen, j = read_leb128(body, 0)
            name = body[j : j + nlen]
        sections.append((sec_id, start, body, name))

    _, _, spec_body, _ = next(s for s in sections if s[3] == SPEC_SECTION)
    nlen, j = read_leb128(spec_body, 0)
    entries = spec_body[j + nlen :]

    with tempfile.NamedTemporaryFile(suffix=".bin") as tmp:
        tmp.write(entries)
        tmp.flush()
        decoded = subprocess.run(
            ["stellar", "xdr", "decode", "--type", "ScSpecEntry", "--input", "stream",
             "--output", "json", tmp.name],
            capture_output=True, text=True, check=True,
        ).stdout

    stripped_lines = [
        json.dumps(blank_docs(entry), separators=(",", ":"))
        for entry in (json.loads(line) for line in decoded.splitlines() if line.strip())
        if "udt_error_enum_v0" not in entry
    ]

    with tempfile.NamedTemporaryFile(suffix=".json", mode="w") as tmp:
        tmp.write("\n".join(stripped_lines) + "\n")
        tmp.flush()
        encoded = subprocess.run(
            ["stellar", "xdr", "encode", "--type", "ScSpecEntry", "--input", "json",
             "--output", "stream", tmp.name],
            capture_output=True, check=True,
        ).stdout

    rebuilt = bytearray(wasm[:8])
    for sec_id, _, body, name in sections:
        if name == META_SECTION:
            continue
        if name == SPEC_SECTION:
            body = write_leb128(len(SPEC_SECTION)) + SPEC_SECTION + encoded
        rebuilt += bytes([sec_id]) + write_leb128(len(body)) + body
    open(dst, "wb").write(bytes(rebuilt))
    print(f"{src}: {len(wasm)} -> {len(rebuilt)} bytes (spec docs + meta stripped)")


if __name__ == "__main__":
    main()
