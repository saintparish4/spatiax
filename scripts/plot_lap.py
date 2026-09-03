#!/usr/bin/env python3
"""Plot a lap from spatiax CSV output.

    spatiax decode --format csv car.dbc lap.log > lap.csv
    python3 scripts/plot_lap.py lap.csv -o lap.png

One panel per channel on a shared lap-time axis, so channels with different
units never share a scale. The defaults match the demo database in
fixtures/demo; pass --channels to plot any set of signals from your own log.

Needs matplotlib: pip install matplotlib
"""

import argparse
import csv
import sys
from dataclasses import dataclass, field
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
from matplotlib.ticker import MaxNLocator  # noqa: E402

DEFAULT_CHANNELS = [
    "Speed",
    "EngineRPM",
    "Gear",
    "ThrottlePos",
    "BrakePressureFront",
    "SteeringAngle",
    "LatAccel",
]
DEFAULT_LABELLED = ["Speed", "BrakePressureFront", "LatAccel"]

THEMES = {
    "light": {
        "surface": "#fcfcfb",
        "ink": "#0b0b0b",
        "secondary": "#52514e",
        "muted": "#898781",
        "grid": "#e1e0d9",
        "baseline": "#c3c2b7",
        "series": "#2a78d6",
    },
    "dark": {
        "surface": "#1a1a19",
        "ink": "#ffffff",
        "secondary": "#c3c2b7",
        "muted": "#898781",
        "grid": "#2c2c2a",
        "baseline": "#383835",
        "series": "#3987e5",
    },
}

DPI = 160
WIDTH_IN = 10.0
PANEL_IN = 1.0
HEADER_IN = 0.95


def px(n):
    """Matplotlib sizes in points; I think in output pixels."""
    return n * 72 / DPI


@dataclass
class Channel:
    name: str
    unit: str
    t: list = field(default_factory=list)
    v: list = field(default_factory=list)


def read_channels(path):
    """Every signal's samples, plus the frame count for the subtitle."""
    channels = {}
    frames = set()
    with open(path, newline="") as handle:
        for row in csv.DictReader(handle):
            frames.add((row["timestamp"], row["id"]))
            channel = channels.setdefault(
                row["signal"], Channel(row["signal"], row["unit"])
            )
            channel.t.append(float(row["timestamp"]))
            channel.v.append(float(row["value"]))
    return channels, len(frames)


def is_stepped(values):
    """Integer channels with a handful of states, like gear, read as steps."""
    return all(v == int(v) for v in values) and len(set(values)) <= 12


def style_axes(ax, theme, stepped):
    ax.set_facecolor(theme["surface"])
    for side in ("top", "right", "left"):
        ax.spines[side].set_visible(False)
    ax.spines["bottom"].set_color(theme["baseline"])
    ax.spines["bottom"].set_linewidth(px(1))
    ax.grid(True, axis="y", color=theme["grid"], linewidth=px(1))
    ax.set_axisbelow(True)
    ax.tick_params(colors=theme["muted"], labelsize=7.5, length=0, pad=3)
    ax.yaxis.set_major_locator(MaxNLocator(nbins=3, integer=stepped))


def set_range(ax, values, theme, labelled):
    lo, hi = min(values), max(values)
    headroom = 1.2 if labelled else 1.1
    if lo >= 0:
        ax.set_ylim(0, (hi or 1) * headroom)
        return
    span = max(abs(lo), abs(hi)) * headroom
    ax.set_ylim(-span, span)
    ax.axhline(0, color=theme["baseline"], linewidth=px(1), zorder=1)


def mark_extreme(ax, t, channel, theme):
    """One marker and one label on the peak; the axis carries the rest."""
    i = max(range(len(channel.v)), key=lambda k: abs(channel.v[k]))
    ax.plot(
        t[i],
        channel.v[i],
        "o",
        markersize=px(9),
        color=theme["series"],
        markeredgecolor=theme["surface"],
        markeredgewidth=px(2),
        zorder=3,
        clip_on=False,
    )
    near_end = t[i] > 0.85 * t[-1]
    ax.annotate(
        f"{channel.v[i]:.3g} {channel.unit}".strip(),
        (t[i], channel.v[i]),
        xytext=(-8 if near_end else 8, 0),
        textcoords="offset points",
        fontsize=8,
        color=theme["secondary"],
        ha="right" if near_end else "left",
        va="center",
    )


def draw_panel(ax, channel, t0, theme, labelled):
    t = [x - t0 for x in channel.t]
    stepped = is_stepped(channel.v)
    style_axes(ax, theme, stepped)
    ax.plot(
        t,
        channel.v,
        color=theme["series"],
        linewidth=px(2),
        drawstyle="steps-post" if stepped else "default",
        solid_joinstyle="round",
        zorder=2,
    )
    set_range(ax, channel.v, theme, labelled)
    title = f"{channel.name} ({channel.unit})" if channel.unit else channel.name
    ax.set_title(title, loc="left", fontsize=9, color=theme["ink"], pad=4)
    if labelled:
        mark_extreme(ax, t, channel, theme)


def render(channels, names, labelled, theme, title, subtitle, out):
    fig, axes = plt.subplots(
        len(names),
        1,
        sharex=True,
        figsize=(WIDTH_IN, HEADER_IN + PANEL_IN * len(names)),
        dpi=DPI,
    )
    fig.patch.set_facecolor(theme["surface"])
    t0 = min(channels[name].t[0] for name in names)
    t_end = max(channels[name].t[-1] for name in names) - t0
    for ax, name in zip(axes, names):
        draw_panel(ax, channels[name], t0, theme, name in labelled)
    axes[-1].set_xlim(0, t_end)
    axes[-1].set_xlabel("Lap time (s)", fontsize=8, color=theme["muted"])
    header = HEADER_IN / fig.get_figheight()
    fig.text(0.04, 1 - 0.34 * header, title, fontsize=13, color=theme["ink"])
    fig.text(0.04, 1 - 0.62 * header, subtitle, fontsize=8.5, color=theme["secondary"])
    fig.subplots_adjust(left=0.06, right=0.97, top=1 - header, bottom=0.55 / fig.get_figheight(), hspace=0.7)
    fig.savefig(out, dpi=DPI, facecolor=theme["surface"])


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("csv", type=Path, help="output of spatiax decode --format csv")
    parser.add_argument("-o", "--output", type=Path, default=Path("lap.png"))
    parser.add_argument("--theme", choices=THEMES, default="light")
    parser.add_argument(
        "--channels",
        default=",".join(DEFAULT_CHANNELS),
        help="comma-separated signal names, one panel each, top to bottom",
    )
    parser.add_argument(
        "--label",
        default=",".join(DEFAULT_LABELLED),
        help="comma-separated signals whose peak gets a marker and a value",
    )
    parser.add_argument("--title", default="One lap, decoded by spatiax")
    parser.add_argument("--subtitle", help="defaults to a summary of the CSV")
    return parser.parse_args()


def main():
    args = parse_args()
    channels, frames = read_channels(args.csv)
    names = args.channels.split(",")
    missing = [name for name in names if name not in channels]
    if missing:
        print(f"not in {args.csv}: {', '.join(missing)}", file=sys.stderr)
        print(f"available: {', '.join(sorted(channels))}", file=sys.stderr)
        return 2
    first = min(channel.t[0] for channel in channels.values())
    last = max(channel.t[-1] for channel in channels.values())
    subtitle = args.subtitle or (
        f"{args.csv.name}: {frames:,} frames decoded into "
        f"{len(channels)} channels over {last - first:.1f} s"
    )
    labelled = set(args.label.split(",")) if args.label else set()
    render(channels, names, labelled, THEMES[args.theme], args.title, subtitle, args.output)
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
