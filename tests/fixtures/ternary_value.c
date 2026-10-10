static int pick(int a, int b) {
    int m = a > b ? a : b;
    return m;
}

/* Non-constant arms keep clang -O0 on the phi path (constant arms become `select`). */
static int absval(int x) {
    return x > 0 ? x : (x < 0 ? -x : 0);
}

int main(void) {
    int s = pick(3, 9) + pick(7, 2);
    return s + absval(5) + absval(-4) + absval(0) + 1;
}