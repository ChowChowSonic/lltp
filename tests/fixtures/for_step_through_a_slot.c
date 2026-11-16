int main(void) {
    int i = 0;
    int *p = &i;
    for (i = 0; i < 3; i = i + 1) {
        *p = *p + 2;
    }
    return i;
}
