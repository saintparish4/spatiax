#!/usr/bin/env python3
"""Reference decoder for the differential test in tests/differential.rs.

For every `<name>.dbc` in the given directory, reads `<name>.frames` (one
`<message name> <hex payload>` per line) and writes `<name>.expected`:

    S <frame index> <signal name> <raw> <physical value>
    L <frame index> <signal name> <label>   (the value table names this raw)
    E <frame index> <hex payload re-encoded from the raw values>
    U <frame index> <multiplexor value>   (no m<N> signal claims this value)

Everything printed is cantools' own output, untouched. Raw values for signed
signals are therefore negative integers, and physical values are Python ints
when the DBC scaling is integral. The Rust side is responsible for comparing
across that representation gap. A label runs to the end of its line and may
contain spaces or be empty.

For a multiplexed message the S lines cover only the signals cantools
considered present. cantools refuses to decode a frame whose multiplexor
value selects no page; such frames get a U line carrying the value it saw.

Labels come from `Signal.choices`, the tables cantools parsed, looked up by
the raw value it unpacked -- which is exactly what its `decode_data` does
with `scaling=False`. Decoding with `decode_choices=True` instead would go
raw -> label -> number to select a multiplexor page, and lands on the wrong
page when two keys of the multiplexor share a label.
"""

import pathlib
import re
import sys

import cantools
from cantools.database.errors import DecodeError

UNCLAIMED = re.compile(r"expected multiplexer id .*, but got (-?\d+)$")


def process(dbc_path: pathlib.Path) -> int:
    db = cantools.database.load_file(dbc_path, strict=True)
    frames = dbc_path.with_suffix(".frames").read_text().splitlines()
    lines = []
    cases = 0

    for index, frame in enumerate(frames):
        name, hexdata = frame.split()
        message = db.get_message_by_name(name)
        data = bytes.fromhex(hexdata)

        try:
            raw = message.decode(data, decode_choices=False, scaling=False)
        except DecodeError as error:
            unclaimed = UNCLAIMED.search(str(error))
            if unclaimed is None:
                raise
            lines.append(f"U {index} {unclaimed.group(1)}")
            cases += 1
            continue

        scaled = message.decode(data, decode_choices=False, scaling=True)
        for signal_name in raw:
            lines.append(f"S {index} {signal_name} {raw[signal_name]} {scaled[signal_name]!r}")
            cases += 1
            choices = message.get_signal_by_name(signal_name).choices
            if choices and raw[signal_name] in choices:
                lines.append(f"L {index} {signal_name} {choices[raw[signal_name]]}")

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
