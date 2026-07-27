#include <stdint.h>
#include <stdio.h>

extern int64_t nivren_double(const int64_t *arguments, uint8_t *overflow);

int main(void) {
    const int64_t arguments[] = {21};
    uint8_t overflow = 0;
    const int64_t result = nivren_double(arguments, &overflow);
    if (overflow != 0) {
        return 2;
    }
    printf("%lld\n", (long long)result);
    return result == 42 ? 0 : 1;
}
