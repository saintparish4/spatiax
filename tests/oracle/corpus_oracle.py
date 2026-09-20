#!/usr/bin/env python3
"""Reference reading of a corpus of real DBC files, for tests/corpus.rs.

Loads every `*.dbc` in the given directory with `cantools` and writes one
line-oriented record per file, message and signal to stdout. The format is
deliberately flat: the Rust side parses it by hand, the way it parses the
differential oracle's output, so the tests need no JSON dependency.

    F <file> <ok> <error text>          one per file, lenient load
    K <file> <ok> <error text>          one per file, strict load
    M <file> <id> <extended> <dlc> <name>
    S <file> <id> <start> <len> <order> <sign> <factor> <offset> <mux> <name>

`ok` is 1 or 0. `order` is B for big endian (Motorola) or L for little
endian (Intel); `sign` is S or U; `mux` is `M` for a multiplexor, `m<N>` for
a page, or `-`. The signal name comes last because it is the only field that
could in principle carry a space. Floats are written with `repr`, which
round-trips, and the reader parses them rather than comparing the text.

Two loads per file on purpose. The lenient one is what the structural
comparison uses. The strict one records whether cantools would refuse the
file over a layout problem, which is what `dbc::check` claims to find.
"""

import pathlib
import sys

import cantools


def describe(signal) -> str:
    order = "B" if signal.byte_order == "big_endian" else "L"
    sign = "S" if signal.is_signed else "U"
    if signal.is_multiplexer:
        mux = "M"
    elif signal.multiplexer_ids:
        mux = "m" + ",".join(str(i) for i in sorted(signal.multiplexer_ids))
    else:
        mux = "-"
    factor = repr(float(signal.scale))
    offset = repr(float(signal.offset))
    return f"{signal.start} {signal.length} {order} {sign} {factor} {offset} {mux} {signal.name}"


def load(path: pathlib.Path, strict: bool):
    try:
        return cantools.database.load_file(path, strict=strict), ""
    except Exception as error:  # cantools raises several types; all mean "no"
        return None, " ".join(str(error).split())


def main() -> None:
    directory = pathlib.Path(sys.argv[1])
    out = []
    for path in sorted(directory.glob("*.dbc")):
        name = path.name
        db, error = load(path, strict=False)
        out.append(f"F {name} {0 if db is None else 1} {error}")
        _, strict_error = load(path, strict=True)
        out.append(f"K {name} {0 if strict_error else 1} {strict_error}")
        if db is None:
            continue
        for message in db.messages:
            extended = 1 if message.is_extended_frame else 0
            out.append(
                f"M {name} {message.frame_id} {extended} {message.length} {message.name}"
            )
            for signal in message.signals:
                out.append(f"S {name} {message.frame_id} {describe(signal)}")
    sys.stdout.write("\n".join(out) + "\n")
    print(f"corpus_oracle: cantools={cantools.__version__}", file=sys.stderr)


if __name__ == "__main__":
    main()
