// NeonMix.exe: entry point of the installed app. Keeps per-user data under
// %LOCALAPPDATA%\NeonMix, points the bundled GStreamer runtimes at the install
// directory and starts the desktop UI without a console window.
#define UNICODE
#define _UNICODE
#include <windows.h>
#include <shlobj.h>
#include <wchar.h>

static void fail(const wchar_t *message) {
    MessageBoxW(NULL, message, L"NeonMix", MB_OK | MB_ICONERROR);
    ExitProcess(1);
}

static void join(wchar_t *out, const wchar_t *base, const wchar_t *tail) {
    if (_snwprintf_s(out, MAX_PATH, _TRUNCATE, L"%ls\\%ls", base, tail) < 0) {
        fail(L"安装路径过长。");
    }
}

static void set_path_env(const wchar_t *name, const wchar_t *base, const wchar_t *tail) {
    wchar_t value[MAX_PATH];
    join(value, base, tail);
    SetEnvironmentVariableW(name, value);
}

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE previous, PWSTR arguments, int show) {
    (void)instance; (void)previous; (void)show;
    wchar_t exe[MAX_PATH], root[MAX_PATH], local[MAX_PATH], state[MAX_PATH], temp[MAX_PATH];
    wchar_t desktop[MAX_PATH], bin[MAX_PATH];
    if (!GetModuleFileNameW(NULL, exe, MAX_PATH)) fail(L"无法定位 NeonMix.exe。");
    wcscpy_s(root, MAX_PATH, exe);
    wchar_t *slash = wcsrchr(root, L'\\');
    if (!slash) fail(L"无法定位安装目录。");
    *slash = L'\0';
    if (FAILED(SHGetFolderPathW(NULL, CSIDL_LOCAL_APPDATA, NULL, SHGFP_TYPE_CURRENT, local))) {
        fail(L"无法定位 %LOCALAPPDATA%。");
    }
    join(state, local, L"NeonMix");
    join(temp, state, L"tmp");
    if (!CreateDirectoryW(state, NULL) && GetLastError() != ERROR_ALREADY_EXISTS) fail(L"无法创建数据目录。");
    if (!CreateDirectoryW(temp, NULL) && GetLastError() != ERROR_ALREADY_EXISTS) fail(L"无法创建临时目录。");

    join(bin, root, L"bin");
    join(desktop, bin, L"neonmix-desktop.exe");
    SetEnvironmentVariableW(L"TMP", temp);
    SetEnvironmentVariableW(L"TEMP", temp);
    SetEnvironmentVariableW(L"TMPDIR", temp);
    set_path_env(L"GST_PLUGIN_SYSTEM_PATH_1_0", root, L"plugins");
    SetEnvironmentVariableW(L"GST_PLUGIN_PATH_1_0", L"");
    SetEnvironmentVariableW(L"GST_PLUGIN_PATH", L"");
    set_path_env(L"GST_REGISTRY", state, L"gstreamer-registry.bin");
    // The MSVC and MinGW GStreamer builds cannot share one plugin scanner
    // process; scan in-process instead.
    SetEnvironmentVariableW(L"GST_REGISTRY_FORK", L"no");
    set_path_env(L"NEONMIX_AIRPLAY_RUNTIME", root, L"airplay");
    SetEnvironmentVariableW(L"NEONMIX_CJK_FONT", NULL);
    wchar_t old_path[32767];
    DWORD length = GetEnvironmentVariableW(L"PATH", old_path, 32767);
    wchar_t *path = (wchar_t *)HeapAlloc(GetProcessHeap(), 0, (length + MAX_PATH + 2) * sizeof(wchar_t));
    if (!path) fail(L"内存不足。");
    swprintf(path, length + MAX_PATH + 2, L"%ls%ls%ls", bin, length ? L";" : L"", length ? old_path : L"");
    SetEnvironmentVariableW(L"PATH", path);
    HeapFree(GetProcessHeap(), 0, path);

    size_t command_length = wcslen(desktop) + wcslen(state) + wcslen(arguments) + 32;
    wchar_t *command = (wchar_t *)HeapAlloc(GetProcessHeap(), 0, command_length * sizeof(wchar_t));
    if (!command) fail(L"内存不足。");
    swprintf(command, command_length, L"\"%ls\" --state-dir \"%ls\" %ls", desktop, state, arguments);

    STARTUPINFOW startup = {sizeof(startup)};
    PROCESS_INFORMATION process = {0};
    if (!CreateProcessW(desktop, command, NULL, NULL, FALSE, CREATE_NO_WINDOW, NULL, state, &startup, &process)) {
        fail(L"无法启动 NeonMix 桌面程序。请重新安装。");
    }
    CloseHandle(process.hThread);
    CloseHandle(process.hProcess);
    return 0;
}
