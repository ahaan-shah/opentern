"""Runs one Opentern script, answering JSON requests on stdin (one per
line) with JSON replies on stdout. Started by the app; not for direct use."""

import io
import json
import os
import sys
import traceback

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import opentern  # noqa: E402
from opentern import _state  # noqa: E402


def load(path, inputs, n=0):
    _state.reset(inputs)
    _state.n = n
    with open(path, encoding="utf-8") as f:
        src = f.read()
    g = {"__name__": "__opentern_script__", "__file__": path}
    exec(compile(src, path, "exec"), g)
    return g


def main():
    path = sys.argv[1]
    # Replies get the pipe to themselves: what the script's libraries or
    # other programs print goes nowhere.
    proto = os.fdopen(os.dup(1), "w", encoding="utf-8")
    null = os.open(os.devnull, os.O_WRONLY)
    os.dup2(null, 1)
    os.close(null)
    for line in sys.stdin:
        try:
            req = json.loads(line)
        except ValueError:
            continue
        log = io.StringIO()
        sys.stdout = log
        try:
            if req.get("cmd") == "describe":
                load(path, {})
                resp = {"ok": True, "meta": _state.meta()}
            elif req.get("cmd") == "run":
                bars = opentern.Bars(req["bars"])
                g = load(path, req.get("inputs", {}), len(bars))
                fn = g.get("compute")
                if fn is None:
                    raise RuntimeError("The script needs a compute(bars) function.")
                fn(bars)
                resp = {"ok": True, "meta": _state.meta(), "out": _state.output(len(bars))}
            else:
                resp = {"ok": False, "error": "unknown request"}
        except Exception:
            resp = {"ok": False, "error": traceback.format_exc(limit=6)}
        finally:
            sys.stdout = sys.__stdout__
        resp["log"] = log.getvalue()[-4000:]
        try:
            text = json.dumps(resp, default=str, allow_nan=False)
        except ValueError:
            text = json.dumps({"ok": False, "error": "A setting or input of this script is nan or infinite."})
        proto.write(text + "\n")
        proto.flush()


if __name__ == "__main__":
    main()
