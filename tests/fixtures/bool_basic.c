#include <stdbool.h>
int main(void) {
    bool flag = true;
    int n = 0;
    while (flag) {
        n++;
        if (n >= 10) flag = false;
    }
    return n;
}