"""Run a console program under a Windows pseudo console (ConPTY) with ctypes, no dependencies.
Pty(cmdline, cols, rows, env) -> .read() blocking bytes from the program, .write(bytes) to its keyboard,
.resize(cols, rows), .alive(), .close()."""
import ctypes
import threading
from ctypes import wintypes

k32 = ctypes.WinDLL("kernel32", use_last_error=True)
HANDLE = ctypes.c_void_p
EXTENDED_STARTUPINFO_PRESENT = 0x00080000
CREATE_UNICODE_ENVIRONMENT = 0x00000400
PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE = 0x00020016
USE_STD_FLAG = True


class COORD(ctypes.Structure):
    _fields_ = [("X", wintypes.SHORT), ("Y", wintypes.SHORT)]


class STARTUPINFOW(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("lpReserved", wintypes.LPWSTR), ("lpDesktop", wintypes.LPWSTR),
                ("lpTitle", wintypes.LPWSTR), ("dwX", wintypes.DWORD), ("dwY", wintypes.DWORD),
                ("dwXSize", wintypes.DWORD), ("dwYSize", wintypes.DWORD), ("dwXCountChars", wintypes.DWORD),
                ("dwYCountChars", wintypes.DWORD), ("dwFillAttribute", wintypes.DWORD), ("dwFlags", wintypes.DWORD),
                ("wShowWindow", wintypes.WORD), ("cbReserved2", wintypes.WORD), ("lpReserved2", ctypes.c_void_p),
                ("hStdInput", HANDLE), ("hStdOutput", HANDLE), ("hStdError", HANDLE)]


class STARTUPINFOEXW(ctypes.Structure):
    _fields_ = [("StartupInfo", STARTUPINFOW), ("lpAttributeList", ctypes.c_void_p)]


class PROCESS_INFORMATION(ctypes.Structure):
    _fields_ = [("hProcess", HANDLE), ("hThread", HANDLE), ("dwProcessId", wintypes.DWORD),
                ("dwThreadId", wintypes.DWORD)]


k32.CreatePipe.argtypes = [ctypes.POINTER(HANDLE), ctypes.POINTER(HANDLE), ctypes.c_void_p, wintypes.DWORD]
k32.CreatePipe.restype = wintypes.BOOL
k32.CreatePseudoConsole.argtypes = [COORD, HANDLE, HANDLE, wintypes.DWORD, ctypes.POINTER(HANDLE)]
k32.CreatePseudoConsole.restype = ctypes.c_long
k32.ResizePseudoConsole.argtypes = [HANDLE, COORD]
k32.ResizePseudoConsole.restype = ctypes.c_long
k32.ClosePseudoConsole.argtypes = [HANDLE]
k32.ClosePseudoConsole.restype = None
k32.InitializeProcThreadAttributeList.argtypes = [ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(ctypes.c_size_t)]
k32.InitializeProcThreadAttributeList.restype = wintypes.BOOL
k32.UpdateProcThreadAttribute.argtypes = [ctypes.c_void_p, wintypes.DWORD, ctypes.c_size_t, ctypes.c_void_p,
                                          ctypes.c_size_t, ctypes.c_void_p, ctypes.c_void_p]
k32.UpdateProcThreadAttribute.restype = wintypes.BOOL
k32.CreateProcessW.argtypes = [wintypes.LPCWSTR, wintypes.LPWSTR, ctypes.c_void_p, ctypes.c_void_p, wintypes.BOOL,
                               wintypes.DWORD, ctypes.c_void_p, wintypes.LPCWSTR, ctypes.POINTER(STARTUPINFOEXW),
                               ctypes.POINTER(PROCESS_INFORMATION)]
k32.CreateProcessW.restype = wintypes.BOOL
k32.ReadFile.argtypes = [HANDLE, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p]
k32.ReadFile.restype = wintypes.BOOL
k32.WriteFile.argtypes = [HANDLE, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p]
k32.WriteFile.restype = wintypes.BOOL
k32.CloseHandle.argtypes = [HANDLE]
k32.CloseHandle.restype = wintypes.BOOL
k32.WaitForSingleObject.argtypes = [HANDLE, wintypes.DWORD]
k32.WaitForSingleObject.restype = wintypes.DWORD
k32.TerminateProcess.argtypes = [HANDLE, wintypes.UINT]
k32.TerminateProcess.restype = wintypes.BOOL


class Pty:
    def __init__(self, cmdline, cols, rows, env=None):
        self.cols, self.rows = cols, rows
        self.closed = False
        self.wlock = threading.Lock()
        in_read, in_write, out_read, out_write = HANDLE(), HANDLE(), HANDLE(), HANDLE()
        if not k32.CreatePipe(ctypes.byref(in_read), ctypes.byref(in_write), None, 0): raise OSError("CreatePipe in")
        if not k32.CreatePipe(ctypes.byref(out_read), ctypes.byref(out_write), None, 0): raise OSError("CreatePipe out")
        self.hpc = HANDLE()
        hr = k32.CreatePseudoConsole(COORD(cols, rows), in_read, out_write, 0, ctypes.byref(self.hpc))
        if hr != 0: raise OSError("CreatePseudoConsole hr=%d" % hr)
        k32.CloseHandle(in_read); k32.CloseHandle(out_write)
        self.in_write, self.out_read = in_write, out_read
        size = ctypes.c_size_t(0)
        k32.InitializeProcThreadAttributeList(None, 1, 0, ctypes.byref(size))
        self.attrs = ctypes.create_string_buffer(size.value)
        if not k32.InitializeProcThreadAttributeList(self.attrs, 1, 0, ctypes.byref(size)): raise OSError("InitAttrList")
        if not k32.UpdateProcThreadAttribute(self.attrs, 0, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, self.hpc,
                                             ctypes.sizeof(HANDLE), None, None): raise OSError("UpdateAttr")
        si = STARTUPINFOEXW()
        si.StartupInfo.cb = ctypes.sizeof(STARTUPINFOEXW)
        si.lpAttributeList = ctypes.cast(self.attrs, ctypes.c_void_p)
        if USE_STD_FLAG:
            si.StartupInfo.dwFlags = 0x100
        pi = PROCESS_INFORMATION()
        env_block = None
        if env is not None:
            block = "".join("%s=%s\0" % (k, v) for k, v in env.items()) + "\0"
            env_block = ctypes.create_unicode_buffer(block, len(block))
        cmd = ctypes.create_unicode_buffer(cmdline)
        flags = EXTENDED_STARTUPINFO_PRESENT | (CREATE_UNICODE_ENVIRONMENT if env_block is not None else 0)
        ok = k32.CreateProcessW(None, cmd, None, None, False, flags,
                                ctypes.cast(env_block, ctypes.c_void_p) if env_block is not None else None, None,
                                ctypes.byref(si), ctypes.byref(pi))
        if not ok: raise OSError("CreateProcessW err=%d" % ctypes.get_last_error())
        self.hproc, self.pid = pi.hProcess, pi.dwProcessId
        k32.CloseHandle(pi.hThread)

    def read(self, n=65536):
        buf = ctypes.create_string_buffer(n)
        got = wintypes.DWORD(0)
        if not k32.ReadFile(self.out_read, buf, n, ctypes.byref(got), None): return b""
        return buf.raw[:got.value]

    def write(self, data):
        if self.closed or not data: return
        with self.wlock:
            n = wintypes.DWORD(0)
            k32.WriteFile(self.in_write, data, len(data), ctypes.byref(n), None)

    def resize(self, cols, rows):
        if (cols, rows) != (self.cols, self.rows) and not self.closed:
            k32.ResizePseudoConsole(self.hpc, COORD(cols, rows)); self.cols, self.rows = cols, rows

    def alive(self):
        return not self.closed and k32.WaitForSingleObject(self.hproc, 0) == 258  # WAIT_TIMEOUT

    def close(self):
        if self.closed: return
        self.closed = True
        try: k32.TerminateProcess(self.hproc, 1)
        except Exception: pass
        try: k32.ClosePseudoConsole(self.hpc)
        except Exception: pass
        for h in (self.in_write, self.out_read, self.hproc):
            try: k32.CloseHandle(h)
            except Exception: pass
