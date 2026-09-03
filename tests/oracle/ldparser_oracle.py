"""Read a `.ld` file with `ldparser` and print what it found.

The counterpart of `cantools_oracle.py`: an implementation nobody on this
side wrote, reporting in a form the Rust can compare exactly. Sample values
are printed as the bits of the float they decoded to, not as decimals,
because a decimal rendering would put the comparison at the mercy of two
float formatters rather than of the file.

Usage: ldparser_oracle.py <file.ld>
"""

import struct
import sys

import numpy as np
from ldparser import read_ldfile


def main(path):
    head, channels = read_ldfile(path)
    stamp = head.datetime.strftime("%d/%m/%Y %H:%M:%S")
    print(f"H {head.driver}|{head.vehicleid}|{head.venue}|{stamp}|{len(channels)}")
    for index, channel in enumerate(channels):
        dtype = np.dtype(channel.dtype).name
        print(
            f"C {index} {channel.name}|{channel.short_name}|{channel.unit}"
            f"|{channel.freq}|{channel.data_len}|{dtype}"
        )
        for position, value in enumerate(channel.data):
            bits = struct.unpack("<I", struct.pack("<f", float(value)))[0]
            print(f"V {index} {position} {bits}")


if __name__ == "__main__":
    main(sys.argv[1])
