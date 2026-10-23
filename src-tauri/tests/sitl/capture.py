#!/usr/bin/env python3
"""Record real Betaflight SITL CLI/MSP responses as test fixtures.

    tests/sitl/capture.py <betaflight_SITL.elf> <outdir>

Starts SITL in a scratch dir (fresh eeprom.bin), talks to UART1 over TCP 5761
(the SITL TCP serial bridge: UARTn = 5760+n), records MSP v1 identity replies,
then a CLI transcript. Output files:
  msp.txt           <NAME> <code> <reply payload hex>
  version.txt serial.txt feature.txt dump_all.txt diff_all.txt   raw CLI output
  transcript.txt    ">>> cmd" followed by the raw response, for every command
  save.txt          batch+save / failed-batch+save / exit behaviour across restarts
"""
import os, re, shutil, socket, subprocess, sys, tempfile, time

PORT = 5761
MSP = [("API_VERSION", 1), ("FC_VARIANT", 2), ("FC_VERSION", 3), ("BOARD_INFO", 4),
       ("BUILD_INFO", 5), ("UID", 160)]


class Sitl:
    def __init__(self, binary, wd):
        self.binary, self.wd, self.p, self.s = os.path.abspath(binary), wd, None, None

    def start(self):
        # Fail fast if another SITL (e.g. Device's smoke test on the shared box) holds the
        # port: we would otherwise capture from the wrong process.
        for _ in range(30):
            try:
                socket.create_connection(("127.0.0.1", PORT), timeout=0.2).close()
            except OSError:
                break
            time.sleep(0.1)
        else:
            raise SystemExit("TCP %d is already in use (another SITL running?); stop it first" % PORT)
        self.p = subprocess.Popen([self.binary], cwd=self.wd, stdout=subprocess.DEVNULL,
                                  stderr=subprocess.DEVNULL)
        for _ in range(100):
            try:
                self.s = socket.create_connection(("127.0.0.1", PORT), timeout=1)
                time.sleep(0.5)
                return
            except OSError:
                time.sleep(0.1)
        raise SystemExit("SITL did not open TCP %d" % PORT)

    def alive(self):
        return self.p.poll() is None

    def stop(self):
        if self.s:
            self.s.close()
        if self.p and self.p.poll() is None:
            self.p.kill()
            self.p.wait()

    def read(self, until=b"\r\n# ", timeout=10.0):
        buf, end = b"", time.time() + timeout
        self.s.settimeout(0.2)
        while time.time() < end:
            try:
                d = self.s.recv(65536)
            except socket.timeout:
                continue
            except OSError:
                break
            if not d:
                break
            buf += d
            if until and buf.endswith(until):
                break
        return buf

    def enter_cli(self):
        for _ in range(40):
            self.s.sendall(b"#")
            if b"Entering CLI" in self.read(timeout=0.5):
                self.read(timeout=0.3)
                return
        raise SystemExit("no CLI prompt")

    def msp(self, code):
        self.s.sendall(b"$M<" + bytes([0, code, code]))
        raw = self.read(until=None, timeout=0.6)
        if len(raw) < 6 or raw[:3] != b"$M>":
            return None
        n = raw[3]
        return raw[5:5 + n]

    def cli(self, line, timeout=10.0):
        self.s.sendall(line.encode() + b"\n")
        return self.read(timeout=timeout)


LINE = re.compile(rb"^(#.*|set [A-Za-z0-9_]+ = .*|feature -?[A-Z0-9_]+|serial \S+ \d+ \d+ \d+ \d+ \d+|"
                  rb"aux( \d+){7}|adjrange( \d+){9,10}|rxrange( \d+){3}|rxfail \d+ [ahs]( \d+)?|"
                  rb"(board_name|manufacturer_id|mcu_id|signature)( \S*)?|mixer \S+|mmix reset|smix reset|"
                  rb"map [A-Z0-9]+|(profile|rateprofile|battery_profile) \d|waypoint .*|batch (start|end)|defaults nosave|save|"
                  rb"(dump|diff) all|serial|feature|resource .*|beacon .*|vtxtable .*|vtx( \d+)+|led .*|color .*|"
                  rb"mode_color .*|timer .*|dma .*|servo .*|)$")


def well_formed(text):
    lines = text.replace(b"\r", b"").split(b"\n")
    if not lines or not text.rstrip().endswith(b"#"):
        return False
    seen = set()
    for ln in lines:
        if not LINE.match(ln):
            return False
        if ln.startswith((b"# profile", b"# rateprofile", b"# battery_profile")):
            seen = set()
        if ln.startswith(b"set "):
            key = ln.split(b" ")[1]
            if key in seen:
                return False
            seen.add(key)
    return True


def commands(dialect_2025):
    rx_port, bad_port = ("UART2", "UART9") if dialect_2025 else ("1", "40")
    return [
        "get serialrx_provider", "get motor_idle", "get dshot_idle_value", "get craft_name",
        "bogus_cmd", "set foo = 1", "set serialrx_provider = NOPE",
        "set serialrx_provider = CRSF", "set serialrx_provider = SBUS",
        "feature GPS", "feature -GPS", "feature NOPE", "feature RX_MSP",
        f"serial {rx_port} 64 115200 57600 0 115200",
        f"serial {bad_port} 64 115200 57600 0 115200",
        f"serial {rx_port} 0 115200 57600 0 115200",
        "aux 0 0 0 1700 2100 0 0", "aux 99 0 0 1700 2100 0 0", "aux 0 0 0 900 900 0 0",
        "batch start", "batch end", "resource", "name",
    ]


def main():
    binary, out = sys.argv[1], sys.argv[2]
    os.makedirs(out, exist_ok=True)
    wd = tempfile.mkdtemp(prefix="bfsw-sitl-")
    fc = Sitl(binary, wd)
    try:
        fc.start()
        with open(os.path.join(out, "msp.txt"), "w") as f:
            for name, code in MSP:
                r = fc.msp(code)
                f.write(f"{name} {code} {'' if r is None else r.hex()}\n")
        fc.enter_cli()
        ver = fc.cli("version")
        m = re.search(rb"\) (\d+)\.(\d+)\.(\d+) ", ver)
        is2025 = bool(m) and int(m.group(1)) >= 2025
        raw = {"version": ver}
        for c in ["serial", "feature", "dump all", "diff all"]:
            # SITL's TCP bridge occasionally drops/duplicates bytes on large outputs;
            # retry until the text is structurally clean.
            for attempt in range(20):
                raw[c] = fc.cli(c, timeout=15)
                if c not in ("dump all", "diff all") or well_formed(raw[c]):
                    break
            else:
                raise SystemExit(f"could not capture a clean '{c}'")
        for k, v in raw.items():
            with open(os.path.join(out, k.replace(" ", "_") + ".txt"), "wb") as f:
                f.write(v)
        with open(os.path.join(out, "transcript.txt"), "wb") as f:
            for c in commands(is2025):
                f.write(b">>> " + c.encode() + b"\n" + fc.cli(c) + b"\n")
        # save / exit semantics across restarts
        log = []
        def step(c):
            try:
                _step(c)
            except OSError:
                log.append(b">>> " + c.encode() + b"\n<connection closed by FC>\n")
        def _step(c):
            if not fc.alive():
                r = b"<process exited>"
            elif c in ("save", "exit"):
                # Betaflight answers then reboots; there is no prompt to wait for.
                fc.s.sendall(c.encode() + b"\n")
                r = fc.read(until=None, timeout=2.0)
            else:
                r = fc.cli(c, timeout=3)
            log.append(b">>> " + c.encode() + b"\n" + r + b"\n")
        def restart(tag):
            time.sleep(1.0)
            log.append(b"--- " + tag.encode() + b": process alive=" + str(fc.alive()).encode() + b"\n")
            fc.stop()
            fc.start()
            fc.enter_cli()
        for c in ["batch start", "set serialrx_provider = CRSF", "batch end", "save"]:
            step(c)
        restart("after save")
        step("get serialrx_provider")
        for c in ["batch start", "set serialrx_provider = SBUS", "set foo = 1", "batch end", "save"]:
            step(c)
        restart("after failed-batch save")
        step("get serialrx_provider")
        step("set serialrx_provider = SBUS")
        step("exit")
        restart("after exit")
        step("get serialrx_provider")
        with open(os.path.join(out, "save.txt"), "wb") as f:
            f.write(b"".join(log))
    finally:
        fc.stop()
        shutil.rmtree(wd, ignore_errors=True)


if __name__ == "__main__":
    main()
