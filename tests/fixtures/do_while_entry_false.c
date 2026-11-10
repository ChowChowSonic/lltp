int main(void) {
    int i = 5;
    int total = 0;
    do {
        total = total + i;
        i = i - 1;
    } while (i > 0 && i < 3);
    return total;
}
