#!/usr/bin/env python3
"""Claude Code 호환 래퍼.

Claude Code 2.1.x 는 initialize 없이 `_meta.protocolVersion=2026-07-28` 을 실어
tools/list 를 보낸다. rmcp 3.x 는 그 피어에게 `resultType` 을 붙여 응답하는데,
Claude Code 는 `resultType` 이 붙은 결과에 `ttlMs`(number)·`cacheScope`
("public"|"private")까지 요구하고 없으면 응답 전체를 버린다. 도구가 0개로 잡힌다.

이 래퍼는 서버 stdout 을 훑어 `result.resultType` 이 있는데 두 필드가 없는 응답에
보수적인 기본값을 채워 넣는다. 그 외 바이트는 건드리지 않는다.

rmcp 나 Claude Code 중 한쪽이 고쳐지면 이 래퍼는 필요 없다.
"""
import json
import os
import subprocess
import sys
import threading

TTL_MS = int(os.environ.get("MCP_COMPAT_TTL_MS", "60000"))
CACHE_SCOPE = os.environ.get("MCP_COMPAT_CACHE_SCOPE", "private")


def patch(line: bytes) -> bytes:
    stripped = line.strip()
    if not stripped.startswith(b"{"):
        return line
    try:
        msg = json.loads(stripped)
    except (ValueError, UnicodeDecodeError):
        return line
    result = msg.get("result")
    if not isinstance(result, dict) or "resultType" not in result:
        return line
    changed = False
    if not isinstance(result.get("ttlMs"), (int, float)):
        result["ttlMs"] = TTL_MS
        changed = True
    if result.get("cacheScope") not in ("public", "private"):
        result["cacheScope"] = CACHE_SCOPE
        changed = True
    if not changed:
        return line
    return (json.dumps(msg, ensure_ascii=False) + "\n").encode()


def main() -> int:
    target = os.environ.get("MCP_COMPAT_TARGET")
    argv = [target] + sys.argv[1:] if target else sys.argv[1:]
    if not argv:
        sys.stderr.write("MCP_COMPAT_TARGET 또는 인자로 실행할 서버를 지정한다\n")
        return 2

    child = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE)

    def pump_stdin():
        # 클라이언트 -> 서버는 손대지 않고 그대로 흘린다
        try:
            for chunk in iter(lambda: sys.stdin.buffer.readline(), b""):
                child.stdin.write(chunk)
                child.stdin.flush()
        except (BrokenPipeError, ValueError):
            pass
        finally:
            try:
                child.stdin.close()
            except OSError:
                pass

    threading.Thread(target=pump_stdin, daemon=True).start()

    for line in iter(child.stdout.readline, b""):
        sys.stdout.buffer.write(patch(line))
        sys.stdout.buffer.flush()
    rc = child.wait()
    # 데몬 스레드가 stdin 락을 쥔 채 인터프리터가 내려가면 종료 시 시끄럽다
    os._exit(rc)


if __name__ == "__main__":
    sys.exit(main())
