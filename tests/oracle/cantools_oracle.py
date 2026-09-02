#!/usr/bin/env python3
"""Reference decoder for the differential test in tests/differential.rs.

For every `<name>.dbc` in the given directory, reads `<name>.frames` (one
`<message name> <hex payload>` per line) and writes `<name>.expected`:

    S <frame index> <signal name> <raw> <physical value>
    E <frame index> <hex payload re-encoded from the raw values>

Everything printed is cantools' own output, untouched. Raw values for signed
signals are therefore negative integers, and physical values are Python ints
when the DBC scaling is integral. The Rust side is responsible for comparing
across that representation gap.
"""

import pathlib
import sys

import cantools


def process(dbc_path: pathlib.Path) -> int:
    db = cantools.database.load_file(dbc_path, strict=True)
    frames = dbc_path.with_suffix(".frames").read_text().splitlines()
    lines = []
    cases = 0

    for index, frame in enumerate(frames):
        name, hexdata = frame.split()
        message = db.get_message_by_name(name)
        data = bytes.fromhex(hexdata)

        raw = message.decode(data, decode_choices=False, scaling=False)
        scaled = message.decode(data, decode_choices=False, scaling=True)
        for signal in message.signals:
            lines.append(f"S {index} {signal.name} {raw[signal.name]} {scaled[signal.name]!r}")
            cases += 1

        encoded = message.encode(raw, scaling=False, padding=False, strict=True)
        lines.append(f"E {index} {encoded.hex()}")

    dbc_path.with_suffix(".expected").write_text("\n".join(lines) + "\n")
    return cases


def main() -> None:
    directory = pathlib.Path(sys.argv[1])
    total = sum(process(path) for path in sorted(directory.glob("*.dbc")))
    print(f"cantools={cantools.__version__} cases={total}")


if __name__ == "__main__":
    main()
