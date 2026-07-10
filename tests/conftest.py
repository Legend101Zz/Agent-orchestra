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


@pytest.fixture
def fake_pi_json(tmp_path, monkeypatch):
    """Fake pi for -p --mode json emitting the real event protocol."""
    bindir = tmp_path / "fakebin-json"
    bindir.mkdir()
    script = bindir / "pi"
    script.write_text(
        "#!/usr/bin/env bash\n"
        "echo '{\"type\":\"agent_start\"}'\n"
        "echo '{\"type\":\"message_update\",\"assistantMessageEvent\":{\"type\":\"text_delta\",\"contentIndex\":1,\"delta\":\"json part one \"}}'\n"
        "echo '{\"type\":\"message_update\",\"assistantMessageEvent\":{\"type\":\"text_delta\",\"contentIndex\":1,\"delta\":\"json part two\"}}'\n"
        "echo '{\"type\":\"agent_end\",\"messages\":[{\"role\":\"assistant\",\"usage\":{\"input\":120,\"output\":30,\"cacheRead\":2048,\"totalTokens\":2198,\"cost\":{\"total\":0.000201}}}]}'\n"
    )
    script.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    return script


@pytest.fixture
def fake_pi_rpc(tmp_path, monkeypatch):
    """Fake pi for --mode rpc emitting the real event protocol (verified 2026-07-10)."""
    bindir = tmp_path / "fakebin-rpc"
    bindir.mkdir()
    script = bindir / "pi"
    script.write_text(
        "#!/usr/bin/env bash\n"
        "read -r line\n"
        "echo '{\"type\":\"response\",\"command\":\"prompt\",\"success\":true}'\n"
        "echo '{\"type\":\"agent_start\"}'\n"
        'if [[ "$line" == *HANG* ]]; then\n'
        "  echo '{\"type\":\"message_update\",\"assistantMessageEvent\":{\"type\":\"text_delta\",\"contentIndex\":1,\"delta\":\"hanging...\"}}'\n"
        "  sleep 30\n"
        "fi\n"
        "echo '{\"type\":\"message_update\",\"assistantMessageEvent\":{\"type\":\"text_delta\",\"contentIndex\":1,\"delta\":\"part one \"}}'\n"
        "echo '{\"type\":\"message_update\",\"assistantMessageEvent\":{\"type\":\"text_delta\",\"contentIndex\":1,\"delta\":\"part two\"}}'\n"
        "echo '{\"type\":\"agent_end\",\"messages\":[{\"role\":\"assistant\",\"usage\":{\"input\":84,\"output\":19,\"cacheRead\":1536,\"totalTokens\":1639,\"cost\":{\"total\":0.00014016}}}]}'\n"
    )
    script.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    return script
