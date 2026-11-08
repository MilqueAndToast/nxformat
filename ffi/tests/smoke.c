/* Links against the built library through include/nxformat.h only.
 * Usage: smoke <archive.nx>. Prints the @SIZE line and exits 0 on success. */
#include "nxformat.h"
#include <stdio.h>
#include <string.h>

static void on_line(void* user, int is_error, const char* line) {
    int* found = (int*)user;
    if (!is_error && strncmp(line, "@SIZE\t", 6) == 0) {
        printf("%s\n", line);
        *found = 1;
    }
}

int main(int argc, char** argv) {
    if (argc != 2) return 2;
    if (nxformat_abi_version() != NXFORMAT_ABI_VERSION) {
        fprintf(stderr, "ABI mismatch\n");
        return 1;
    }
    printf("nxformat %s\n", nxformat_version());
    const char* args[] = {"contents", argv[1], "--frontend"};
    nxformat_cancel* cancel = nxformat_cancel_new();
    char error[512] = {0};
    int found = 0;
    /* --frontend reads stdin for cancellation only without a handle. */
    int code = nxformat_run(args, 3, on_line, &found, cancel, error, sizeof error);
    nxformat_cancel_free(cancel);
    if (code != NXFORMAT_OK || !found) {
        fprintf(stderr, "run failed (%d): %s\n", code, error);
        return 1;
    }
    return 0;
}
