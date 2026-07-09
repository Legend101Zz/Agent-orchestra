import os
import stat
import sys
from pathlib import Path

import pytest


@pytest.fixture
def orc_home(tmp_path, monkeypatch):
    home = tmp_path / "orchestra"
    monkeypatch.setenv("ORC_HOME", str(home))
    return home


@pytest.fixture
def fake_pi(tmp_path, monkeypatch):
    """A stand-in `pi` on PATH. Echoes a canned reply; sleeps when task contains SLEEP."""
    bindir = tmp_path / "fakebin"
    bindir.mkdir()
    script = bindir / "pi"
    script.write_text(
        "#!/usr/bin/env bash\n"
        'task="${@: -1}"\n'
        'if [[ "$task" == *SLEEP* ]]; then echo "sleeping"; sleep 30; fi\n'
        'echo "FAKE-PI-REPLY: $task"\n'
    )
    script.chmod(script.stat().st_mode | stat.S_IEXEC)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    return script
