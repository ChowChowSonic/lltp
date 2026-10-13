static int both(int a, int b) {
    int r = (a > 0 && b > 0);
    return r;
}

static int either(int a, int b) {
    int r = (a > 0 || b > 0);
    return r;
}

static int mixed(int a, int b, int c) {
    return (a > 0 && b > 0) || c > 0;
}

int main(void) {
    int code = 0;
    code += both(1, 1) * 1;
    code += both(1, 0) * 2;
    code += both(0, 1) * 4;
    code += either(0, 0) * 8;
    code += either(0, 1) * 16;
    code += either(1, 0) * 32;
    code += mixed(0, 0, 1) * 64;
    return code;
}