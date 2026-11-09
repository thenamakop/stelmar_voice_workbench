"""pytest setup for eval/: puts eval/ on sys.path, isolates every test from Track A's real lock files, and
makes `heavy` tests wait for the gate request flag and the gate lock."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

from wbeval import common  # noqa: E402
from wbeval.common import set_below_normal_priority, wait_for_gate_lock, wait_for_gate_request  # noqa: E402

# The real files Track A uses, remembered before any test redirects them.
REAL_LOCKS = {"GATE_REQUEST": common.GATE_REQUEST, "GATE_LOCK": common.GATE_LOCK, "BUSY_LOCK": common.BUSY_LOCK}


@pytest.fixture(autouse=True)
def isolated_locks(request: pytest.FixtureRequest, tmp_path_factory: pytest.TempPathFactory,
                   monkeypatch: pytest.MonkeyPatch):
    """Point the gate request flag, gate lock and busy lock at an empty temporary folder for every
    unit test, so no test can ever see (and wait or spin on) Track A's real files. Tests marked
    `heavy` keep the real paths: they must wait for Track A."""
    if request.node.get_closest_marker("heavy"):
        yield
        return
    folder = tmp_path_factory.mktemp("locks")
    for name, env in (("GATE_REQUEST", "WB_GATE_REQUEST"), ("GATE_LOCK", "WB_GATE_LOCK"), ("BUSY_LOCK", "WB_BUSY_LOCK")):
        path = folder / REAL_LOCKS[name].name
        monkeypatch.setattr(common, name, path)
        monkeypatch.setenv(env, str(path))
    yield


def pytest_runtest_setup(item: pytest.Item) -> None:
    if item.get_closest_marker("heavy"):
        # Tests that decode audio share the CPU with Track A's timing-sensitive gate runs.
        wait_for_gate_request(f"test {item.name}")
        wait_for_gate_lock(f"test {item.name}")
        set_below_normal_priority()
