#!/usr/bin/env python3
"""HTTPS-only owner tool. Invitation codes go to a new private file."""
import argparse
import http.client
import json
import os
from pathlib import Path
import ssl
import stat
from urllib.parse import urlsplit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--origin", required=True)
    parser.add_argument("--owner-token", required=True, type=Path)
    parser.add_argument("--mutation", type=Path, help="JSON mutation; omit to read directory")
    parser.add_argument("--invitation-out", type=Path, help="required for invite; new 0600 JSON file")
    parser.add_argument("--enrollments", action="store_true", help="list enrollment and revoked device states")
    args = parser.parse_args()
    origin = urlsplit(args.origin)
    if origin.scheme != "https" or not origin.hostname or origin.username or origin.password or origin.path not in ("", "/") or origin.query or origin.fragment:
        raise ValueError("origin must be an HTTPS origin")
    descriptor = os.open(args.owner_token, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor) as source:
        metadata = os.fstat(source.fileno())
        if not stat.S_ISREG(metadata.st_mode) or stat.S_IMODE(metadata.st_mode) != 0o600 or metadata.st_uid != os.getuid() or metadata.st_nlink != 1:
            raise ValueError("credential must be a private regular file")
        secret = source.read(65)
    if len(secret) != 64 or any(c not in "0123456789abcdef" for c in secret):
        raise ValueError("invalid credential")
    mutation = None
    if args.mutation:
        if args.mutation.stat().st_size > 128 * 1024:
            raise ValueError("mutation exceeds 128 KiB")
        mutation = json.loads(args.mutation.read_text())
    invite = mutation is not None and mutation.get("op") == "invite"
    if args.enrollments and mutation is not None:
        raise ValueError("listing does not accept a mutation")
    if invite != (args.invitation_out is not None):
        raise ValueError("invite requires --invitation-out; other requests must omit it")
    if invite:
        if mutation.get("controller", args.origin).rstrip("/") != args.origin.rstrip("/"):
            raise ValueError("invitation controller must match HTTPS origin")
        mutation["controller"] = args.origin
    enrollment_operation = mutation is not None and mutation.get("op") in ("invite", "approve", "reject")
    endpoint = "/v1/enrollment/admin" if enrollment_operation else "/v1/mutations"
    if mutation is None:
        endpoint = "/v1/enrollment" if args.enrollments else "/v1/directory"
    # Reserve before sending so an occupied output path cannot lose a new token.
    output = None
    if args.invitation_out:
        output = os.open(args.invitation_out, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    committed = False
    try:
        connection = http.client.HTTPSConnection(origin.hostname, origin.port or 443, context=ssl.create_default_context(), timeout=10)
        try:
            connection.request("POST" if mutation else "GET", endpoint,
                               body=json.dumps(mutation) if mutation else None,
                               headers={"Authorization": f"Bearer {secret}", "Content-Type": "application/json"})
            response = connection.getresponse()
            # No redirects, including redirects to another HTTPS origin.
            if response.status != 200:
                raise ValueError(f"controller rejected request (HTTP {response.status})")
            data = response.read(2 * 1024 * 1024 + 1)
            if len(data) > 2 * 1024 * 1024:
                raise ValueError("controller response exceeds limit")
            result = json.loads(data)
        finally:
            connection.close()
        if invite:
            invitation = result.get("invitation", {})
            credential = invitation.get("code", "")
            if len(credential) != 64 or any(c not in "0123456789abcdef" for c in credential):
                raise ValueError("invalid invitation response")
            with os.fdopen(output, "w") as destination:
                output = None
                json.dump(invitation, destination)
                destination.flush()
                os.fsync(destination.fileno())
            committed = True
            parent = os.open(args.invitation_out.parent, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(parent)
            finally:
                os.close(parent)
            del result["invitation"]
        print(json.dumps(result, indent=2))
    finally:
        if output is not None:
            os.close(output)
        if args.invitation_out and not committed:
            args.invitation_out.unlink(missing_ok=True)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, http.client.HTTPException):
        # Never echo exception messages, credentials, submitted bodies or URLs.
        raise SystemExit("Registry request failed; check HTTPS, private credential, revision and output path") from None
