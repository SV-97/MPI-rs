#include <sys/time.h>
#include <stdio.h>

#include "loh.h"

double loh_wtime(void)
{
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return tv.tv_sec + 0.000001 * tv.tv_usec;
}