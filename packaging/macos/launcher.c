// NeonMix.app entry point. Resolves the bundle from its own path, keeps all
// per-user data under ~/Library, wires the bundled GStreamer runtime and starts
// the desktop binary as a child so the registered .app stays the responsible
// process for LaunchServices/TCC (microphone) attribution.
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <mach-o/dyld.h>
#include <pwd.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

static void joined(char *out, const char *root, const char *suffix) {
    if (snprintf(out, PATH_MAX, "%s/%s", root, suffix) >= PATH_MAX) {
        fputs("NeonMix: path is too long\n", stderr);
        exit(1);
    }
}

static void make_dir(const char *path) {
    if (mkdir(path, 0700) && errno != EEXIST) {
        fprintf(stderr, "NeonMix: cannot create %s: %s\n", path, strerror(errno));
        exit(1);
    }
}

static void set_env(const char *name, const char *root, const char *suffix) {
    char value[PATH_MAX];
    joined(value, root, suffix);
    if (setenv(name, value, 1)) {
        perror("NeonMix: setenv");
        exit(1);
    }
}

int main(int argc, char **argv) {
    char executable[PATH_MAX], bundle[PATH_MAX], home[PATH_MAX];
    char library[PATH_MAX], support[PATH_MAX], cache[PATH_MAX], logs[PATH_MAX], path[PATH_MAX];
    uint32_t size = sizeof(executable);
    if (_NSGetExecutablePath(executable, &size) || !realpath(executable, bundle)) {
        perror("NeonMix: executable path");
        return 1;
    }
    // <bundle>/Contents/MacOS/NeonMix -> <bundle>
    for (int i = 0; i < 3; ++i) {
        char *slash = strrchr(bundle, '/');
        if (!slash) return 1;
        *slash = '\0';
    }
    const char *user_home = getenv("HOME");
    if (!user_home || !*user_home) {
        struct passwd *entry = getpwuid(getuid());
        user_home = entry ? entry->pw_dir : NULL;
    }
    if (!user_home) return 1;
    snprintf(home, sizeof(home), "%s", user_home);

    umask(077);
    joined(library, home, "Library");
    joined(support, library, "Application Support/NeonMix");
    joined(cache, library, "Caches/NeonMix");
    joined(logs, library, "Logs/NeonMix");
    make_dir(library);
    joined(path, library, "Application Support");
    make_dir(path);
    make_dir(support);
    joined(path, library, "Caches");
    make_dir(path);
    make_dir(cache);
    joined(path, library, "Logs");
    make_dir(path);
    make_dir(logs);

    joined(path, logs, "desktop.log");
    int log = open(path, O_WRONLY | O_CREAT | O_APPEND, 0600);
    if (log >= 0) {
        dup2(log, STDOUT_FILENO);
        dup2(log, STDERR_FILENO);
        close(log);
    }

    setenv("TMPDIR", cache, 1);
    setenv("TMP", cache, 1);
    setenv("TEMP", cache, 1);
    set_env("GST_PLUGIN_SYSTEM_PATH_1_0", bundle, "Contents/Resources/plugins");
    set_env("GST_PLUGIN_SCANNER", bundle, "Contents/Helpers/gst-plugin-scanner");
    set_env("GST_PLUGIN_SCANNER_1_0", bundle, "Contents/Helpers/gst-plugin-scanner");
    set_env("GST_REGISTRY", cache, "gstreamer-registry.bin");
    set_env("NEONMIX_AIRPLAY_RUNTIME", bundle, "Contents/Resources/airplay");
    setenv("GST_PLUGIN_PATH_1_0", "", 1);
    setenv("GST_PLUGIN_PATH", "", 1);
    unsetenv("NEONMIX_CJK_FONT");
    if (chdir(support)) return 1;

    joined(executable, bundle, "Contents/MacOS/bin/neonmix-desktop");
    char **arguments = calloc((size_t)argc + 3, sizeof(*arguments));
    if (!arguments) return 1;
    int count = 0;
    arguments[count++] = executable;
    arguments[count++] = "--state-dir";
    arguments[count++] = support;
    for (int i = 1; i < argc; ++i) {
        if (strncmp(argv[i], "-psn_", 5)) arguments[count++] = argv[i];
    }
    pid_t child = 0;
    int error = posix_spawn(&child, executable, NULL, NULL, arguments, environ);
    free(arguments);
    if (error) {
        fprintf(stderr, "NeonMix: desktop launch failed: %s\n", strerror(error));
        return 1;
    }
    int status = 0;
    while (waitpid(child, &status, 0) < 0) {
        if (errno != EINTR) return 1;
    }
    return WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
}
