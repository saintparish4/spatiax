#!/usr/bin/env python3
"""Generate the synthetic lap in `fixtures/demo/`.

The lap is scripted, not recorded. A point-mass car drives one flying lap of
a fictional 3.5 km circuit at GT3-like limits, and every channel — engine,
pedals, wheel speeds, accelerations, dampers, tyre temperatures, lap timing —
comes from that model. `cantools` encodes each frame from
`fixtures/demo/gt3.dbc`, so the log is the reference implementation's own
encoding of the scripted lap, and reading it back is spatiax's job.

    pip install cantools==43.0.2
    python3 scripts/synthetic_lap.py          # rewrite fixtures/demo/synthetic_lap.log
    python3 scripts/synthetic_lap.py --check  # exit 1 if the committed log differs

The output is deterministic: a fixed seed drives the sensor noise and the
model has no other source of variation, so `--check` can run in CI.
"""

import math
import random
import sys
from dataclasses import dataclass
from pathlib import Path

import cantools

ROOT = Path(__file__).resolve().parent.parent
DBC = ROOT / "fixtures" / "demo" / "gt3.dbc"
LOG = ROOT / "fixtures" / "demo" / "synthetic_lap.log"

SEED = 20260903
START_EPOCH = 1_756_900_800.0  # 2025-09-03 12:00:00 UTC, arbitrary
INTERFACE = "can0"
LAP_NUMBER = 7
FUEL_AT_LINE = 61.5  # litres
FUEL_PER_LAP = 2.8

G = 9.81
DT = 0.001  # simulation step, seconds
LAT_LIMIT = 1.5 * G  # cornering, m/s²
BRAKE_LIMIT = 1.4 * G
TRACTION_LIMIT = 7.0  # low-speed acceleration, m/s²
POWER_PER_MASS = 330.0  # W/kg
TOP_SPEED = 78.0  # m/s; sets the drag coefficient
DRAG = POWER_PER_MASS / TOP_SPEED**3
CORNER_DRAG = 0.15  # extra deceleration per m/s² of lateral load
WHEELBASE = 2.65
TRACK_WIDTH = 1.65
STEERING_RATIO = 13.5
SHIFT_TIME = 0.045
REDLINE = 8800
UPSHIFT_RPM = 8600
DOWNSHIFT_RPM = 5200
# Road speed at the redline in each gear, km/h; index 0 is neutral.
GEAR_SPEEDS = [None, 78, 118, 158, 198, 240, 285]
FRONT_BRAKE_PEAK = 95.0  # bar at full deceleration
BRAKE_BIAS = 56.0

RIGHT, LEFT = 1, -1


@dataclass(frozen=True)
class Straight:
    length: float


@dataclass(frozen=True)
class Corner:
    radius: float
    angle: float  # degrees
    direction: int

    @property
    def length(self):
        return self.radius * math.radians(self.angle)

    @property
    def limit(self):
        return math.sqrt(LAT_LIMIT * self.radius)


# A fictional circuit. The start line is the beginning of the first straight,
# so the car crosses it flat out after the run from the last corner.
TRACK = [
    Straight(600),
    Corner(45, 95, RIGHT),
    Straight(160),
    Corner(65, 70, LEFT),
    Corner(60, 75, RIGHT),
    Straight(430),
    Corner(180, 40, RIGHT),
    Straight(120),
    Corner(85, 110, LEFT),
    Straight(280),
    Corner(32, 170, LEFT),
    Straight(420),
    Corner(210, 55, RIGHT),
    Corner(75, 80, RIGHT),
    Straight(240),
    Corner(55, 90, LEFT),
    Straight(300),
]
LAP_LENGTH = sum(segment.length for segment in TRACK)


@dataclass
class Placed:
    corner: Corner
    start: float
    end: float


def place_corners(laps):
    """Corners with their distance along the track, unrolled over `laps`
    laps so the look-ahead never runs off the end."""
    placed = []
    for lap in range(laps):
        at = lap * LAP_LENGTH
        for segment in TRACK:
            if isinstance(segment, Corner):
                placed.append(Placed(segment, at, at + segment.length))
            at += segment.length
    return placed


# ------------------------------------------------------------------ driving


class Driver:
    """Drives on the allowed-speed curve: flat out until the curve for the
    next corner comes down to meet the car, then on the brakes at the limit,
    then just enough throttle to hold the cornering limit."""

    def __init__(self, corners):
        self.corners = corners

    def current_corner(self, s):
        return next((c for c in self.corners if c.start <= s < c.end), None)

    def allowed_speed(self, s):
        """Fastest speed at `s` from which every corner ahead can still be
        made at its limit."""
        allowed = TOP_SPEED
        for c in self.corners:
            if c.end <= s:
                continue
            gap = max(c.start - s, 0.0)
            allowed = min(allowed, math.sqrt(c.corner.limit**2 + 2 * BRAKE_LIMIT * gap))
        return allowed

    def decide(self, s, v):
        """Returns (longitudinal acceleration, throttle demand 0..1)."""
        corner = self.current_corner(s)
        lateral = v**2 / corner.corner.radius if corner else 0.0
        resist = DRAG * v**2 + CORNER_DRAG * lateral
        drive = min(TRACTION_LIMIT, POWER_PER_MASS / max(v, 1.0))
        # What it takes to be on the curve one step from now.
        needed = (self.allowed_speed(s + v * DT) - v) / DT
        if needed < -0.05:
            return max(needed, -BRAKE_LIMIT), 0.0
        a = min(max(needed, 0.0), drive - resist)
        return a, min((a + resist) / drive, 1.0)


class Gearbox:
    def __init__(self):
        self.gear = 5
        self.shifting = 0.0

    @staticmethod
    def rpm_in(gear, v):
        return REDLINE * v * 3.6 / GEAR_SPEEDS[gear]

    def update(self, v, accelerating):
        self.shifting = max(self.shifting - DT, 0.0)
        rpm = self.rpm_in(self.gear, v)
        if accelerating and rpm >= UPSHIFT_RPM and self.gear < 6:
            self.gear += 1
            self.shifting = SHIFT_TIME
        elif rpm < DOWNSHIFT_RPM and self.gear > 2 and self.rpm_in(self.gear - 1, v) <= 8400:
            self.gear -= 1
            self.shifting = SHIFT_TIME
        return self.rpm_in(self.gear, v)


def smooth(current, target, tau):
    """First-order lag, one step."""
    return current + (target - current) * (DT / tau)


# ------------------------------------------------------------------ sensors


@dataclass
class Tyre:
    front: bool
    left: bool
    middle: float = 78.0

    def load(self, lat, long):
        side = max(lat, 0.0) if self.left else max(-lat, 0.0)
        pitch = max(-long, 0.0) if self.front else max(long, 0.0)
        return 0.4 + 0.8 * side / G + 0.5 * pitch / G

    def update(self, lat, long):
        target = 70.0 + 18.0 * self.load(lat, long)
        self.middle = smooth(self.middle, target, 6.0)

    def readings(self, lat, rng):
        side = max(lat, 0.0) if self.left else max(-lat, 0.0)
        inner = self.middle + 3.0 + rng.gauss(0, 0.05)
        outer = self.middle + 5.0 * side / G + rng.gauss(0, 0.05)
        pressure = 1.72 + 0.006 * (self.middle - 70.0)
        return inner, self.middle + rng.gauss(0, 0.05), outer, pressure


@dataclass
class State:
    t: float = 0.0
    s: float = 0.0
    v: float = 50.0
    a: float = 0.0
    throttle: float = 0.0
    brake: float = 0.0
    steering: float = 0.0
    lat: float = 0.0
    yaw: float = 0.0
    rpm: float = 0.0
    gear: int = 5
    coolant: float = 89.0


class Car:
    def __init__(self, corners, rng):
        self.driver = Driver(corners)
        self.gearbox = Gearbox()
        self.rng = rng
        self.tyres = {
            "FL": Tyre(front=True, left=True),
            "FR": Tyre(front=True, left=False),
            "RL": Tyre(front=False, left=True),
            "RR": Tyre(front=False, left=False),
        }
        self.state = State()

    def step(self):
        st = self.state
        a, throttle = self.driver.decide(st.s, st.v)
        rpm = self.gearbox.update(st.v, throttle > 0)
        if self.gearbox.shifting > 0 and throttle > 0:
            a, throttle = -DRAG * st.v**2, 0.0
        st.v = max(st.v + a * DT, 1.0)
        st.s += st.v * DT
        st.t += DT
        st.a = a
        st.rpm = rpm
        st.gear = self.gearbox.gear
        st.throttle = smooth(st.throttle, 100.0 * throttle, 0.06)
        st.brake = smooth(st.brake, FRONT_BRAKE_PEAK * max(-a, 0.0) / BRAKE_LIMIT, 0.03)
        self.attitude(self.driver.current_corner(st.s))
        st.coolant = smooth(st.coolant, 84.0 + 0.1 * st.throttle, 15.0)
        for tyre in self.tyres.values():
            tyre.update(st.lat, st.a)

    def attitude(self, placed):
        st = self.state
        if placed:
            corner = placed.corner
            lat = corner.direction * st.v**2 / corner.radius
            steer = corner.direction * math.degrees(math.atan(WHEELBASE / corner.radius))
            yaw = corner.direction * math.degrees(st.v / corner.radius)
        else:
            lat = steer = yaw = 0.0
        st.lat = smooth(st.lat, lat, 0.12)
        st.steering = smooth(st.steering, steer * STEERING_RATIO, 0.12)
        st.yaw = smooth(st.yaw, yaw, 0.10)


# ----------------------------------------------------------------- messages


def wheel_speeds(st, rng):
    """Outer wheels run faster than inner ones in a corner, driven wheels
    slip under power, fronts slip under braking."""
    kmh = st.v * 3.6
    turn = st.lat / max(st.v**2, 1.0) * TRACK_WIDTH / 2  # ~ width / (2 radius), signed
    slip_rear = 0.02 * max(st.a, 0.0) / TRACTION_LIMIT
    slip_front = 0.015 * max(-st.a, 0.0) / BRAKE_LIMIT
    left, right = kmh * (1 + turn), kmh * (1 - turn)
    return {
        "WheelSpeedFL": left * (1 - slip_front) + rng.gauss(0, 0.12),
        "WheelSpeedFR": right * (1 - slip_front) + rng.gauss(0, 0.12),
        "WheelSpeedRL": left * (1 + slip_rear) + rng.gauss(0, 0.12),
        "WheelSpeedRR": right * (1 + slip_rear) + rng.gauss(0, 0.12),
    }


def dampers(st, rng):
    """Positive is compression: the outside of a corner, the front under
    braking, the rear under power, plus the road surface."""
    lat, long = st.lat / G, st.a / G
    road = 0.6 * (st.v / 50.0) * (math.sin(2 * math.pi * 11 * st.t) + 0.5 * math.sin(2 * math.pi * 17 * st.t))
    noise = lambda: rng.gauss(0, 0.15)  # noqa: E731
    return {
        "DamperPosFL": 12 * lat - 8 * long + road + noise(),
        "DamperPosFR": -12 * lat - 8 * long + road + noise(),
        "DamperPosRL": 10 * lat + 6 * long + road + noise(),
        "DamperPosRR": -10 * lat + 6 * long + road + noise(),
    }


def engine(st, rng):
    return {
        "EngineRPM": round(st.rpm),
        "ThrottlePos": min(max(st.throttle, 0.0), 100.0),
        "Gear": st.gear,
        "CoolantTemp": round(st.coolant),
        "OilTemp": round(st.coolant + 21),
        "OilPressure": min(max(1.2 + 4.8 * st.rpm / REDLINE + rng.gauss(0, 0.05), 0.0), 12.0),
    }


def pedals(st, rng):
    return {
        "BrakePressureFront": max(st.brake + rng.gauss(0, 0.2), 0.0),
        "BrakePressureRear": max(st.brake * 0.62 + rng.gauss(0, 0.2), 0.0),
        "SteeringAngle": st.steering + rng.gauss(0, 0.3),
        "BrakeBias": BRAKE_BIAS,
    }


def chassis(st, rng):
    return {
        "Speed": st.v * 3.6 + rng.gauss(0, 0.05),
        "LatAccel": st.lat / G + rng.gauss(0, 0.004),
        "LongAccel": st.a / G + rng.gauss(0, 0.004),
        "YawRate": st.yaw + rng.gauss(0, 0.1),
    }


def tyres(car, corner_name):
    st = car.state
    inner, middle, outer, pressure = car.tyres[corner_name].readings(st.lat, car.rng)
    return {
        "TyreCorner": ["FL", "FR", "RL", "RR"].index(corner_name),
        f"TyreTempInner{corner_name}": inner,
        f"TyreTempMiddle{corner_name}": middle,
        f"TyreTempOuter{corner_name}": outer,
        f"TyrePressure{corner_name}": pressure,
    }


def lap(st, lap_start_t):
    distance = st.s % LAP_LENGTH
    return {
        "LapNumber": LAP_NUMBER,
        "LapDistance": distance,
        "LapTime": st.t - lap_start_t,
        "FuelLevel": FUEL_AT_LINE - FUEL_PER_LAP * distance / LAP_LENGTH,
    }


# Period and phase in milliseconds. The ECUs are not synchronised, so each
# message keeps its own phase and the bursts rarely coincide.
SCHEDULE = [
    ("Engine", 20, 0),
    ("Pedals", 20, 7),
    ("WheelSpeeds", 20, 3),
    ("Chassis", 20, 13),
    ("Dampers", 20, 9),
    ("TyreTemps", 50, 5),
    ("Lap", 100, 1),
]
FRAME_TIME = 0.000118  # one classic frame on the bus at 1 Mbit/s


def due(tick):
    return [name for name, period, phase in SCHEDULE if (tick - phase) % period == 0]


def payload(name, car, tick, lap_start_t):
    st, rng = car.state, car.rng
    if name == "TyreTemps":
        return tyres(car, ["FL", "FR", "RL", "RR"][(tick // 50) % 4])
    if name == "Lap":
        return lap(st, lap_start_t)
    return {
        "Engine": engine,
        "Pedals": pedals,
        "WheelSpeeds": wheel_speeds,
        "Chassis": chassis,
        "Dampers": dampers,
    }[name](st, rng)


def generate():
    """The log lines for one flying lap, after a warm-up lap that settles
    the model into steady state."""
    db = cantools.database.load_file(DBC)
    rng = random.Random(SEED)
    car = Car(place_corners(3), rng)
    lines = []
    lap_start_t = None
    tick = 0
    while car.state.s < 2 * LAP_LENGTH:
        car.step()
        tick += 1
        if car.state.s >= LAP_LENGTH and lap_start_t is None:
            lap_start_t = car.state.t
        if lap_start_t is None:
            continue
        for slot, name in enumerate(due(tick)):
            message = db.get_message_by_name(name)
            data = message.encode(payload(name, car, tick, lap_start_t))
            stamp = START_EPOCH + car.state.t - lap_start_t + slot * FRAME_TIME + rng.uniform(0, 25e-6)
            lines.append(f"({stamp:.6f}) {INTERFACE} {message.frame_id:03X}#{data.hex().upper()}")
    summary = f"{len(lines)} frames, lap {car.state.t - lap_start_t:.3f} s, {LAP_LENGTH:.0f} m"
    return "\n".join(lines) + "\n", summary


def main():
    text, summary = generate()
    if "--check" in sys.argv[1:]:
        if not LOG.exists() or LOG.read_text() != text:
            sys.exit(f"{LOG} does not match what this script generates ({summary})")
        print(f"{LOG.relative_to(ROOT)} matches: {summary}")
        return
    LOG.write_text(text)
    print(f"wrote {LOG.relative_to(ROOT)}: {summary}")


if __name__ == "__main__":
    main()
