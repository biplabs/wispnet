"""Durable owner-driven operations; SSH is explicit admin transport, never peer trust."""

from concurrent.futures import ThreadPoolExecutor, as_completed
import fcntl
import json
import os
from pathlib import Path
import re
import secrets
import stat
import subprocess
import tempfile
from .api import NodeAPI, NodeRejected


def identity(value):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value)


def validate_plan(value):
    if not isinstance(value, dict) or set(value) != {"action", "targets"}:
        raise ValueError("Expected action and targets")
    action = value["action"]
    if (
        not isinstance(action, dict)
        or action.get("op") not in ("approve", "revoke")
        or not identity(action.get("peer"))
    ):
        raise ValueError("Expected an explicit client approval or revocation")
    keys = {"op", "peer", "name"} if action["op"] == "approve" else {"op", "peer"}
    if set(action) != keys or (
        action["op"] == "approve"
        and (not isinstance(action["name"], str) or not 0 < len(action["name"]) <= 80)
    ):
        raise ValueError("Invalid client action")
    targets = value["targets"]
    if not isinstance(targets, list) or not 1 <= len(targets) <= 32:
        raise ValueError("Select between one and 32 target hosts")
    seen = set()
    for target in targets:
        if (
            not isinstance(target, dict)
            or set(target) - {"host", "ssh"}
            or not identity(target.get("host"))
            or target["host"] in seen
        ):
            raise ValueError("Invalid or duplicate target host")
        seen.add(target["host"])
        if "ssh" in target and (
            not isinstance(target["ssh"], str)
            or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.@-]{0,253}", target["ssh"])
        ):
            raise ValueError("Invalid explicit SSH owner target")


class Combined:
    def __init__(self, config):
        self.api = NodeAPI(config)
        self.directory = self.api.state / "combined-operations"

    @staticmethod
    def read_private(path):
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(fd, "rb") as source:
            info = os.fstat(source.fileno())
            if (
                not stat.S_ISREG(info.st_mode)
                or info.st_uid != os.getuid()
                or info.st_mode & 0o077
                or info.st_size > 65536
            ):
                raise ValueError(
                    "Operation files must be bounded, owner-only regular files"
                )
            return json.loads(source.read(65537))

    def prepare(self):
        self.directory.mkdir(mode=0o700, exist_ok=True)
        info = self.directory.lstat()
        if (
            not stat.S_ISDIR(info.st_mode)
            or info.st_uid != os.getuid()
            or info.st_mode & 0o077
        ):
            raise ValueError("Unsafe operation directory")

    def path(self, operation):
        if not identity(operation):
            raise ValueError("Invalid operation ID")
        return self.directory / (operation + ".json")

    def save(self, value):
        fd, name = tempfile.mkstemp(prefix="intent-", dir=self.directory)
        try:
            with os.fdopen(fd, "w") as output:
                json.dump(value, output)
                output.flush()
                os.fsync(output.fileno())
            os.replace(name, self.path(value["id"]))
            fd = os.open(self.directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            try:
                os.fsync(fd)
            finally:
                os.close(fd)
        finally:
            Path(name).unlink(missing_ok=True)

    def load(self, operation):
        value = self.read_private(self.path(operation))
        if (
            not isinstance(value, dict)
            or value.get("version") != 1
            or value.get("id") != operation
        ):
            raise ValueError("Invalid saved operation")
        validate_plan(
            {
                "action": value["action"],
                "targets": [
                    {k: v for k, v in target.items() if k in ("host", "ssh")}
                    for target in value["targets"]
                ],
            }
        )
        if any(
            t.get("state") not in ("pending", "applied", "rejected")
            for t in value["targets"]
        ):
            raise ValueError("Invalid saved target status")
        for target in value["targets"]:
            revision = target.get("revision")
            if target["state"] == "applied" and (
                type(revision) is not int or revision < 0
            ):
                raise ValueError("Invalid saved applied receipt")
            if target["state"] != "applied" and revision is not None:
                raise ValueError("Unconfirmed target cannot have an applied revision")
        value["complete"] = all(t["state"] == "applied" for t in value["targets"])
        value["finished"] = all(t["state"] != "pending" for t in value["targets"])
        return value

    def list(self):
        self.prepare()
        files = sorted(self.directory.glob("*.json"))
        if len(files) > 128:
            raise ValueError("Too many operation journals")
        return [self.load(p.stem) for p in files]

    def start(self, plan):
        validate_plan(plan)
        self.prepare()
        if len(list(self.directory.glob("*.json"))) >= 128:
            raise ValueError(
                "Operation journal limit reached; archive finished journals explicitly"
            )
        value = {
            "version": 1,
            "id": secrets.token_hex(32),
            "action": plan["action"],
            "targets": [
                {
                    **target,
                    "state": "pending",
                    "reason": "Not acknowledged",
                    "revision": None,
                }
                for target in plan["targets"]
            ],
            "complete": False,
            "finished": False,
        }
        self.save(
            value
        )  # Intent exists durably before any target can change authority.
        return self.retry(value["id"])

    def attempt(self, value, target):
        request = {
            "op": "apply_operation",
            "id": value["id"],
            "host": target["host"],
            "action": value["action"],
        }
        try:
            if "ssh" in target:
                # This fixed command runs as the already trusted SSH owner; no owner
                # token/capability appears in argv or crosses between host stores.
                result = subprocess.run(
                    [
                        "ssh",
                        "-o",
                        "BatchMode=yes",
                        "-o",
                        "StrictHostKeyChecking=yes",
                        "-o",
                        "ConnectTimeout=4",
                        target["ssh"],
                        '"$HOME/.local/bin/wispnet-node" --response',
                    ],
                    input=json.dumps(request),
                    capture_output=True,
                    text=True,
                    timeout=8,
                )
                try:
                    response = json.loads(result.stdout)
                except ValueError:
                    return {
                        "state": "pending",
                        "reason": "Owner connection unavailable or reply unconfirmed",
                        "revision": None,
                    }
                if response.get("ok") is not True:
                    return {
                        "state": "rejected"
                        if response.get("kind") == "rejected"
                        else "pending",
                        "reason": response.get("error", "Reply unconfirmed"),
                        "revision": None,
                    }
                receipt = response["result"]
            else:
                receipt = self.api.request(**request)
            if (
                receipt.get("host") != target["host"]
                or receipt.get("id") != value["id"]
                or receipt.get("state") not in ("applied", "pending", "rejected")
            ):
                return {
                    "state": "pending",
                    "reason": "Invalid target acknowledgement",
                    "revision": None,
                }
            if receipt["state"] == "applied" and (
                type(receipt.get("revision")) is not int or receipt["revision"] < 0
            ):
                return {
                    "state": "pending",
                    "reason": "Missing durable applied revision",
                    "revision": None,
                }
            return {k: receipt[k] for k in ("state", "revision", "reason")}
        except NodeRejected as error:
            return {"state": "rejected", "reason": str(error), "revision": None}
        except (
            OSError,
            ValueError,
            KeyError,
            TypeError,
            AttributeError,
            subprocess.TimeoutExpired,
        ):
            return {
                "state": "pending",
                "reason": "Target unavailable or reply unconfirmed",
                "revision": None,
            }

    def retry(self, operation):
        self.prepare()
        lock = self.directory / (str(operation) + ".lock")
        self.path(operation)  # Validate before constructing/opening any lock path.
        fd = os.open(lock, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "w") as guard:
            try:
                fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise ValueError("This operation is already running") from None
            value = self.load(operation)
            with ThreadPoolExecutor(max_workers=4) as pool:
                jobs = {
                    pool.submit(self.attempt, value, t): t
                    for t in value["targets"]
                    if t["state"] == "pending"
                }
                for future in as_completed(jobs):
                    jobs[future].update(future.result())
                    value["complete"] = all(
                        t["state"] == "applied" for t in value["targets"]
                    )
                    value["finished"] = all(
                        t["state"] != "pending" for t in value["targets"]
                    )
                    self.save(value)
            return value
