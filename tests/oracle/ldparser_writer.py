"""Rebuild a `.ld` file through `ldparser`'s own writer.

The read-back oracle (`ldparser_oracle.py`) proves that everything a reader
looks at is right. It cannot say anything about the fields a reader skips —
the device identity, the logging magic, the per-channel counter, the
padding — because those never reach it.

So this takes a file, pulls it through `ldparser`'s object model, and writes
it back out with `ldparser`'s writer. Every value the reader understood is
carried over unchanged, which means any byte that differs afterwards is a
byte the reader ignored and the two writers disagree about. Comparing the
result with the original is the closest check available to "does this look
like a file the reference implementation would have produced".

Usage: ldparser_writer.py <input.ld> <output.ld>
"""

import struct
import sys

import numpy as np
from ldparser import ldChan, ldData, ldEvent, ldHead, read_ldfile

HEAD_SIZE = struct.calcsize(ldHead.fmt)
EVENT_SIZE = struct.calcsize(ldEvent.fmt)
CHAN_SIZE = struct.calcsize(ldChan.fmt)


def main(source, destination):
    head, channels = read_ldfile(source)

    event_ptr = HEAD_SIZE
    meta_ptr = HEAD_SIZE + EVENT_SIZE
    data_ptr = meta_ptr + len(channels) * CHAN_SIZE

    read_event = head.event
    event = ldEvent(read_event.name, read_event.session, read_event.comment, 0, None)
    rebuilt_head = ldHead(
        meta_ptr,
        data_ptr,
        event_ptr,
        event,
        head.driver,
        head.vehicleid,
        head.venue,
        head.datetime,
        head.short_comment,
    )

    rebuilt, previous, following = [], 0, meta_ptr + CHAN_SIZE
    for index, channel in enumerate(channels):
        last = index == len(channels) - 1
        copy = ldChan(
            None,
            meta_ptr,
            previous,
            0 if last else following,
            data_ptr,
            channel.data_len,
            np.float32,
            channel.freq,
            0,
            1,
            1,
            0,
            channel.name,
            channel.short_name,
            channel.unit,
        )
        copy._data = np.asarray(channel.data, dtype=np.float32)
        previous, meta_ptr = meta_ptr, following
        following += CHAN_SIZE
        data_ptr += copy._data.nbytes
        rebuilt.append(copy)

    ldData(rebuilt_head, rebuilt).write(destination)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
