"""Optional desktop Command Center adapter; private Wispnet needs none of it."""
import subprocess
import sys
from urllib.parse import parse_qs, urlsplit

NOT_HANDLED = object()
OWNER_OPERATIONS = frozenset({
    'command_center_status', 'command_center_link', 'command_center_poll',
    'command_center_unlink', 'configure_backend_auth',
})


def _base_origin(value):
    if not isinstance(value, str) or len(value) > 2048 or any(ord(c) < 33 for c in value):
        raise ValueError('Invalid Command Center address')
    parsed = urlsplit(value)
    local = parsed.hostname in ('localhost', '127.0.0.1', '::1')
    if (not parsed.hostname or parsed.username or parsed.password
        or parsed.query or parsed.fragment or parsed.path not in ('', '/')
        or not (parsed.scheme == 'https' or local and parsed.scheme == 'http')):
        raise ValueError('Invalid Command Center address')
    # Accessing .port rejects malformed authorities before browser handoff.
    _ = parsed.port
    return parsed


def browser_target(status, purpose):
    base = _base_origin(status.get('base_url'))
    if purpose == 'dashboard':
        return base.geturl().rstrip('/') + '/'
    if purpose != 'link':
        raise ValueError('Unsupported Command Center page')
    pending = status.get('pending')
    url = pending.get('verification_url') if isinstance(pending, dict) else None
    if not isinstance(url, str) or len(url) > 4096 or any(ord(c) < 33 for c in url):
        raise ValueError('Command Center sign-in link unavailable')
    target = urlsplit(url)
    query = parse_qs(target.query, strict_parsing=True)
    if (target.scheme != base.scheme or target.netloc != base.netloc
        or target.path != '/connect' or target.fragment or target.username or target.password
        or set(query) != {'code'} or len(query['code']) != 1
        or not 1 <= len(query['code'][0]) <= 128):
        raise ValueError('Invalid Command Center sign-in link')
    return url


def dispatch(method, args, api):
    if method == 'desktopRequest':
        payload = args.get('payload')
        if isinstance(payload, dict) and payload.get('op') in OWNER_OPERATIONS:
            return api.request(**payload)
    if method == 'launchCommandCenter':
        status = api.request(op='command_center_status')
        url = browser_target(status, args.get('purpose'))
        subprocess.run(['/usr/bin/open' if sys.platform == 'darwin' else 'xdg-open', url],
                       check=True, timeout=15, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return {}
    return NOT_HANDLED
