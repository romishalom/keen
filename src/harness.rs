//! C test runner appended to the entry file's translation unit by `keen test`.
//!
//! Every test runs in a forked child (on Unix) with its output captured, so a failed `assert`,
//! a crash or a non-zero exit only fails that test. Arguments: substrings to filter test names,
//! `--list` to print the names, `--nocapture` to let tests write to the terminal directly.

pub const RUNNER: &str = r##"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#if defined(__unix__) || defined(__APPLE__)
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>
#define KEEN_FORK 1
#endif

static const char **keen_names;
static void (**keen_fns)(void);
static size_t keen_count, keen_cap;

void keen_test_add(const char *name, void (*fn)(void)) {
    if (keen_count == keen_cap) {
        keen_cap = keen_cap ? keen_cap * 2 : 16;
        keen_names = realloc((void *)keen_names, keen_cap * sizeof *keen_names);
        keen_fns = realloc((void *)keen_fns, keen_cap * sizeof *keen_fns);
        if (!keen_names || !keen_fns) {
            fputs("keen: out of memory\n", stderr);
            exit(2);
        }
    }
    keen_names[keen_count] = name;
    keen_fns[keen_count] = fn;
    keen_count++;
}

static int keen_match(const char *name, int nf, char **filters) {
    if (nf == 0) return 1;
    for (int k = 0; k < nf; k++)
        if (strstr(name, filters[k])) return 1;
    return 0;
}

static int keen_run_one(size_t i, int capture) {
#ifdef KEEN_FORK
    int fds[2] = {-1, -1};
    if (capture && pipe(fds) != 0) capture = 0;
    fflush(stdout);
    fflush(stderr);
    pid_t pid = fork();
    if (pid < 0) {
        printf("test %s ... FAILED (could not fork)\n", keen_names[i]);
        return 1;
    }
    if (pid == 0) {
        if (capture) {
            close(fds[0]);
            dup2(fds[1], 1);
            dup2(fds[1], 2);
            close(fds[1]);
        }
        keen_fns[i]();
        fflush(stdout);
        fflush(stderr);
        _exit(0);
    }
    char *buf = NULL;
    size_t len = 0, cap = 0;
    if (capture) {
        close(fds[1]);
        char tmp[4096];
        ssize_t n;
        while ((n = read(fds[0], tmp, sizeof tmp)) > 0) {
            if (len + (size_t)n + 1 > cap) {
                cap = (len + (size_t)n + 1) * 2;
                buf = realloc(buf, cap);
                if (!buf) exit(2);
            }
            memcpy(buf + len, tmp, (size_t)n);
            len += (size_t)n;
        }
        close(fds[0]);
    }
    int status = 0;
    waitpid(pid, &status, 0);
    int ok = WIFEXITED(status) && WEXITSTATUS(status) == 0;
    printf("test %s ... %s\n", keen_names[i], ok ? "ok" : "FAILED");
    if (!ok) {
        if (len) {
            printf("---- %s output ----\n", keen_names[i]);
            fwrite(buf, 1, len, stdout);
            if (buf[len - 1] != '\n') putchar('\n');
        }
        if (WIFSIGNALED(status))
            printf("(terminated by signal %d)\n", WTERMSIG(status));
        else
            printf("(exit code %d)\n", WEXITSTATUS(status));
    }
    free(buf);
    return !ok;
#else
    (void)capture;
    printf("test %s ... ", keen_names[i]);
    fflush(stdout);
    keen_fns[i]();
    printf("ok\n");
    return 0;
#endif
}

static int keen_run_tests(int argc, char **argv) {
    int list = 0, capture = 1, nf = 0;
    char **filters = malloc(((size_t)argc + 1) * sizeof *filters);
    if (!filters) return 2;
    for (int a = 1; a < argc; a++) {
        if (!strcmp(argv[a], "--list")) list = 1;
        else if (!strcmp(argv[a], "--nocapture")) capture = 0;
        else filters[nf++] = argv[a];
    }
    size_t selected = 0, failed = 0;
    for (size_t i = 0; i < keen_count; i++)
        if (keen_match(keen_names[i], nf, filters)) selected++;
    if (list) {
        for (size_t i = 0; i < keen_count; i++)
            if (keen_match(keen_names[i], nf, filters)) printf("%s\n", keen_names[i]);
        free(filters);
        return 0;
    }
    printf("running %zu test%s\n", selected, selected == 1 ? "" : "s");
    for (size_t i = 0; i < keen_count; i++)
        if (keen_match(keen_names[i], nf, filters)) failed += (size_t)keen_run_one(i, capture);
    printf("\ntest result: %s. %zu passed; %zu failed\n", failed ? "FAILED" : "ok", selected - failed, failed);
    free(filters);
    return failed ? 1 : 0;
}
"##;
