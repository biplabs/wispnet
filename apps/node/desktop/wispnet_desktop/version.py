"""Shared release version and packaged source identifier."""
VERSION = "0.2.0"
try:
    from .build_info import DISPLAY_VERSION
except ImportError:
    from pathlib import Path
    import runpy
    DISPLAY_VERSION = runpy.run_path(str(Path(__file__).resolve().parents[4] / "scripts/build_identity.py"))["identity"]()
