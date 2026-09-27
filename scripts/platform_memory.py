"""The memory of a process on this system: `winmem` on Windows, `macmem` on
macOS, `procmem` anywhere else.

All give the same `Proc`. Only the way to find the Flash processes differs:
Windows asks for the `--type=ppapi` processes of EternalTwin, Linux looks for
the processes that map the player, macOS for the ones whose executable is the
player.
"""
import sys

if sys.platform == "win32":
    from winmem import Proc

    def flash_pids(pattern):
        import winmem
        return winmem.ppapi_pids()
elif sys.platform == "darwin":
    from macmem import Proc, flash_pids  # noqa: F401
else:
    from procmem import Proc, flash_pids  # noqa: F401
