"""Paths, logging, hashing, the Track A gate lock and process priority.

Shared by `eval/` and `scripts/models/`. Development time only.
"""

from __future__ import annotations

import ctypes
import hashlib
import logging
import os
import subprocess
import sys
import time
from collections.abc import Sequence
from pathlib import Path

EVAL_DIR = Path(__file__).resolve().parent.parent
REPO = EVAL_DIR.parent
DATA_DIR = EVAL_DIR / "data"
TOOLS_DIR = EVAL_DIR / ".tools"
RESULTS_DIR = EVAL_DIR / "results"
LOG_DIR = EVAL_DIR / "logs"
MODELS_DIR = REPO / "models"
CONVERTED_DIR = MODELS_DIR / "converted"
WHISPER_CPP_DIR = TOOLS_DIR / "whisper.cpp"
OPENAI_WHISPER_DIR = TOOLS_DIR / "openai-whisper"

# Track A (the main session) holds this file while it runs timing-sensitive gates on the Legion
# (soak, load and kill tests). CPU-heavy work here waits while it exists.
GATE_LOCK = Path(os.environ.get("WB_GATE_LOCK", str(REPO.parent / "stelmar-gate.lock")))
GATE_LOCK_POLL_S = 120
# The other direction: this file exists while a heavy step here runs (whisper-cli, conversion,
# quantisation, harness decoding), so Track A can wait for it before starting a gate. It holds the
# step name, PID and start time, is created only after the gate lock is free, and is deleted in a
# finally block. It is never held while waiting, so the two sessions cannot deadlock.
BUSY_LOCK = Path(os.environ.get("WB_BUSY_LOCK", str(REPO.parent / "stelmar-eval-busy.lock")))
# Track A creates this flag when it wants the machine for a gate soon. While it exists no step here
# takes the busy lock (permanent rule, 8 Nov 2026): Track A can then wait for the current step to end
# and start its gate without a new one starting in between.
GATE_REQUEST = Path(os.environ.get("WB_GATE_REQUEST", str(REPO.parent / "stelmar-gate-request.flag")))
# Free RAM needed before a heavy step starts (large-v3-class models need more).
MIN_FREE_GB = 6.0
MIN_FREE_GB_LARGE = 8.0

# Every wait (gate lock, busy lock, memory) is appended here; the harness moves them into its results
# so timings can be read later. Items: {"kind", "step", "started_at", "seconds"}.
WAIT_LOG: list[dict] = []

BELOW_NORMAL_PRIORITY_CLASS = 0x00004000

log = logging.getLogger("wbeval")


def setup_logging(name: str) -> Path:
    """Log to stderr and to eval/logs/<name>.log. Never log tokens or request headers."""
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    path = LOG_DIR / f"{name}.log"
    fmt = logging.Formatter("%(asctime)s %(levelname)s %(message)s", "%Y-%m-%d %H:%M:%S")
    root = logging.getLogger()
    root.setLevel(logging.INFO)
    for handler in (logging.StreamHandler(sys.stderr), logging.FileHandler(path, encoding="utf-8")):
        handler.setFormatter(fmt)
        root.addHandler(handler)
    # Keep HTTP libraries quiet so no URL with a signed query string or header reaches a log.
    for noisy in ("urllib3", "httpx", "huggingface_hub", "filelock"):
        logging.getLogger(noisy).setLevel(logging.WARNING)
    return path


def sha256_file(path: Path, chunk: int = 1 << 20) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while block := f.read(chunk):
            h.update(block)
    return h.hexdigest()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def set_below_normal_priority() -> None:
    """Lower this process to below-normal priority. Children started afterwards inherit it."""
    if sys.platform != "win32":
        os.nice(5)
        return
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.GetCurrentProcess.restype = wintypes.HANDLE  # pseudo-handle -1; must stay 64-bit
    kernel32.SetPriorityClass.argtypes = (wintypes.HANDLE, wintypes.DWORD)
    kernel32.SetPriorityClass.restype = wintypes.BOOL
    if not kernel32.SetPriorityClass(kernel32.GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS):
        log.warning("could not lower process priority (error %d)", ctypes.get_last_error())


def free_ram_gb() -> float:
    """Available physical memory in GB (GlobalMemoryStatusEx on Windows)."""
    if sys.platform != "win32":
        return os.sysconf("SC_AVPHYS_PAGES") * os.sysconf("SC_PAGE_SIZE") / 1e9

    class MemoryStatusEx(ctypes.Structure):
        _fields_ = [("dwLength", ctypes.c_ulong), ("dwMemoryLoad", ctypes.c_ulong),
                    ("ullTotalPhys", ctypes.c_ulonglong), ("ullAvailPhys", ctypes.c_ulonglong),
                    ("ullTotalPageFile", ctypes.c_ulonglong), ("ullAvailPageFile", ctypes.c_ulonglong),
                    ("ullTotalVirtual", ctypes.c_ulonglong), ("ullAvailVirtual", ctypes.c_ulonglong),
                    ("ullAvailExtendedVirtual", ctypes.c_ulonglong)]

    stat = MemoryStatusEx()
    stat.dwLength = ctypes.sizeof(MemoryStatusEx)
    ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(stat))
    return stat.ullAvailPhys / 1e9


def _record_wait(kind: str, step: str, started: float, seconds: float) -> None:
    WAIT_LOG.append({"kind": kind, "step": step, "seconds": round(seconds, 1),
                     "started_at": time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(started))})


def wait_for_memory(step: str, min_free_gb: float, poll_s: int = GATE_LOCK_POLL_S) -> float:
    """Block until at least min_free_gb of RAM is free, logging every check."""
    wall, t0, looped = time.time(), time.perf_counter(), False
    while (free := free_ram_gb()) < min_free_gb:
        looped = True
        log.info("waiting: %.1f GB RAM free, %s needs %.0f GB; paused for %d s so far", free, step, min_free_gb,
                 time.perf_counter() - t0)
        time.sleep(poll_s)
    waited = time.perf_counter() - t0
    if looped:
        _record_wait("memory", step, wall, waited)
    return waited


def _pid_alive(pid: int) -> bool:
    if sys.platform == "win32":
        handle = ctypes.windll.kernel32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
        if not handle:
            return False
        code = ctypes.c_ulong()
        ctypes.windll.kernel32.GetExitCodeProcess(handle, ctypes.byref(code))
        ctypes.windll.kernel32.CloseHandle(handle)
        return code.value == 259  # STILL_ACTIVE
    try:
        os.kill(pid, 0)
    except OSError:
        return False
    return True


def _lock_pid(path: Path) -> int:
    try:
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.startswith("pid:"):
                return int(line.split(":", 1)[1])
    except (OSError, ValueError):
        pass
    return -1


def _take_busy_lock(step: str) -> bool:
    """Create the busy lock atomically. False if another live eval process holds it."""
    body = f"step: {step}\npid: {os.getpid()}\nstarted: {time.strftime('%Y-%m-%d %H:%M:%S')}\n"
    for _ in range(2):
        try:
            fd = os.open(BUSY_LOCK, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        except FileExistsError:
            pid = _lock_pid(BUSY_LOCK)
            if pid > 0 and _pid_alive(pid):
                return False
            log.warning("removing stale %s (pid %s is gone)", BUSY_LOCK, pid)
            BUSY_LOCK.unlink(missing_ok=True)
            continue
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(body)
        return True
    return False


def _release_busy_lock() -> None:
    if _lock_pid(BUSY_LOCK) == os.getpid():
        BUSY_LOCK.unlink(missing_ok=True)


def wait_for_gate_lock(step: str, poll_s: int = GATE_LOCK_POLL_S) -> float:
    """Block while Track A's gate lock exists, logging every check. Call before CPU-heavy steps.
    Returns the seconds spent waiting, so timings can leave them out."""
    wall, start, looped = time.time(), time.perf_counter(), False
    while GATE_LOCK.exists():
        looped = True
        log.info("waiting: %s exists (Track A gate run); %s paused for %d s so far", GATE_LOCK, step,
                 time.perf_counter() - start)
        time.sleep(poll_s)
    waited = time.perf_counter() - start
    if looped:
        log.info("gate lock released after %d s; starting %s", waited, step)
        _record_wait("gate-lock", step, wall, waited)
    return waited


def wait_for_gate_request(step: str, poll_s: int = GATE_LOCK_POLL_S) -> float:
    """Block while Track A's gate-request flag exists, logging every check."""
    wall, start, looped = time.time(), time.perf_counter(), False
    while GATE_REQUEST.exists():
        looped = True
        log.info("waiting: %s exists (Track A asked for the machine); %s paused for %d s so far", GATE_REQUEST,
                 step, time.perf_counter() - start)
        time.sleep(poll_s)
    waited = time.perf_counter() - start
    if looped:
        _record_wait("gate-request", step, wall, waited)
    return waited


def heavy_step(step: str) -> None:
    """Start a CPU-heavy step that runs in this process (dataset resampling): wait for the gate request
    flag, the gate lock and free memory, take the busy lock, and lower this process to below-normal
    priority. The caller must call release_heavy_slot() in a finally block when the step ends."""
    acquire_heavy_slot(step)
    set_below_normal_priority()


def acquire_heavy_slot(step: str, min_free_gb: float = MIN_FREE_GB, poll_s: int = GATE_LOCK_POLL_S) -> float:
    """Wait until the gate lock is gone, enough RAM is free and no other eval step holds the busy lock;
    then take the busy lock. Returns the seconds spent waiting. Pair with release_heavy_slot()."""
    waited = 0.0
    while True:
        waited += wait_for_gate_request(step, poll_s)
        waited += wait_for_gate_lock(step, poll_s)
        waited += wait_for_memory(step, min_free_gb, poll_s)
        if GATE_REQUEST.exists():  # appeared while waiting for the lock or memory: wait again
            continue
        if not _take_busy_lock(step):
            wall, t0 = time.time(), time.perf_counter()
            log.info("waiting: another eval step holds %s; %s waits", BUSY_LOCK, step)
            time.sleep(poll_s)
            _record_wait("busy-lock", step, wall, time.perf_counter() - t0)
            waited += time.perf_counter() - t0
            continue
        if GATE_LOCK.exists() or GATE_REQUEST.exists():  # Track A moved between our check and the lock
            release_heavy_slot()
            continue
        return waited


def release_heavy_slot() -> None:
    _release_busy_lock()


def run_heavy(cmd: Sequence[str | os.PathLike[str]], step: str, min_free_gb: float = MIN_FREE_GB,
              **kwargs) -> subprocess.CompletedProcess:
    """Run a CPU-heavy command at below-normal priority once the gate lock is free and enough RAM is
    free, holding the busy lock while it runs. The result carries `gate_wait_s` (all waiting: gate
    lock, memory, busy lock) and `run_s` (the command itself)."""
    waited = acquire_heavy_slot(step, min_free_gb)
    flags = kwargs.pop("creationflags", 0)
    if sys.platform == "win32":
        flags |= BELOW_NORMAL_PRIORITY_CLASS
    t0 = time.perf_counter()
    try:
        proc = subprocess.run([str(c) for c in cmd], creationflags=flags, **kwargs)
    finally:
        release_heavy_slot()
    proc.gate_wait_s = waited  # type: ignore[attr-defined]
    proc.run_s = time.perf_counter() - t0  # type: ignore[attr-defined]
    return proc


def rel(path: Path) -> str:
    """Path relative to the repository, with forward slashes, for logs and committed files."""
    try:
        return path.resolve().relative_to(REPO).as_posix()
    except ValueError:
        return path.name
