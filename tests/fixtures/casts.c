int main(void) {
    int i = -1;
    unsigned u = (unsigned)i;
    long l = (long)i;
    double d = (double)i;
    char c = (char)300;
    return (int)(u > 5) + (int)l + (int)d + c;
}