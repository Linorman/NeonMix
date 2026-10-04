// NeonMix.exe: entry point of the installed app. Keeps per-user data under
// %LOCALAPPDATA%\NeonMix, points the bundled GStreamer runtimes at the install
// directory and starts the desktop UI without a console window.
#define UNICODE
#define _UNICODE
#include <windows.h>
#include <aclapi.h>
#include <sddl.h>
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

/* The background accepts only directories owned by the user's own SID. An
 * elevated token (the built-in Administrator, or "Run as administrator")
 * defaults new objects to BUILTIN\Administrators, so the owner is explicit. */
static PSID user_sid(void) {
    static DWORD buffer[256];
    HANDLE token;
    DWORD size = sizeof(buffer);
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) fail(L"无法读取当前用户。");
    BOOL ok = GetTokenInformation(token, TokenUser, buffer, size, &size);
    CloseHandle(token);
    if (!ok) fail(L"无法读取当前用户。");
    return ((TOKEN_USER *)buffer)->User.Sid;
}

static void private_dir(const wchar_t *path) {
    PSID user = user_sid();
    wchar_t *sid = NULL, sddl[256];
    if (!ConvertSidToStringSidW(user, &sid)) fail(L"无法读取当前用户。");
    _snwprintf_s(sddl, 256, _TRUNCATE, L"O:%lsD:P(A;OICI;FA;;;%ls)", sid, sid);
    LocalFree(sid);
    SECURITY_ATTRIBUTES attributes = {sizeof(attributes), NULL, FALSE};
    if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl, SDDL_REVISION_1, &attributes.lpSecurityDescriptor, NULL)) {
        fail(L"无法建立数据目录权限。");
    }
    BOOL created = CreateDirectoryW(path, &attributes);
    DWORD error = GetLastError();
    LocalFree(attributes.lpSecurityDescriptor);
    if (created) return;
    if (error != ERROR_ALREADY_EXISTS) fail(L"无法创建数据目录。");
    DWORD flags = GetFileAttributesW(path);
    if (flags == INVALID_FILE_ATTRIBUTES || !(flags & FILE_ATTRIBUTE_DIRECTORY) ||
        (flags & FILE_ATTRIBUTE_REPARSE_POINT)) {
        fail(L"数据目录无效。");
    }
    PSID owner = NULL;
    PSECURITY_DESCRIPTOR descriptor = NULL;
    if (GetNamedSecurityInfoW(path, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, &owner, NULL,
                              NULL, NULL, &descriptor) != ERROR_SUCCESS) {
        fail(L"无法读取数据目录所有者。");
    }
    BOOL mine = EqualSid(owner, user);
    BOOL administrators = IsWellKnownSid(owner, WinBuiltinAdministratorsSid);
    LocalFree(descriptor);
    if (mine) return;
    // Repair a directory an elevated earlier launch created for this user.
    if (!administrators || SetNamedSecurityInfoW((LPWSTR)path, SE_FILE_OBJECT,
                                                 OWNER_SECURITY_INFORMATION, user, NULL, NULL,
                                                 NULL) != ERROR_SUCCESS) {
        fail(L"数据目录不属于当前用户。请删除 %LOCALAPPDATA%\\NeonMix 后重试。");
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
    private_dir(state);
    private_dir(temp);

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
