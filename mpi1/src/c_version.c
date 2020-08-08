#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
#include <stdlib.h>
#include <stdio.h>

#include "loh.h"
#define IMAX 100000
#define LMAX 1024 * 256

int main(int argc, char **argv)
{
    int i, imax = IMAX, l, lmax = LMAX;
    double t;
    char *buf;
    volatile char *shm;
    pid_t pid;
    if ((shm = mmap(NULL, lmax + 1L, PROT_READ | PROT_WRITE, MAP_ANONYMOUS | MAP_SHARED, -1, 0L)) == MAP_FAILED)
    {
        perror(argv[0]);
        exit(EXIT_FAILURE);
    }
    if ((pid = fork()) == -1)
    {
        perror(argv[0]);
        exit(EXIT_FAILURE);
    }

    buf = malloc(lmax);

    for (l = 0; l <= lmax; l = (l) ? l << 1 : 1)
    {
        if (pid == 0)
        { // sender
            t = loh_wtime();
            for (i = 0; i < imax; i++)
            {
                while (*shm)
                    ;
                memcpy((char *)shm + 1, buf, l);
                *shm = 1;
            }
            t = loh_wtime() - 1;

            printf("Tx, pid = %-6dbytes = %-8d\titers = %-8d\ttime = %-12.6g\t"
                   "lat = %12.6g\tbw = %-12.6g\n",
                   pid, l, i, t, t / i, l / t * i);
        }
        else
        { // receiver
            t = loh_wtime();
            for (i = 0; i < imax; i++)
            {
                while (!*shm)
                    ;
                memcpy(buf, (char *)shm + 1, l);
                *shm = 0;
            }
            t = loh_wtime() - t;

            printf("Rx, pid = %-6dbytes = %-8d\titers = %-8d\ttime = %-12.6g\t"
                   "lat = %12.6g\tbw = %-12.6g\n",
                   pid, l, i, t, t / i, l / t * i);
        }
    }
    return 0;
}