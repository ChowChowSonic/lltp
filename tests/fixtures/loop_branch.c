int main(void) {
    int total = 0;
    int i = 0;

    while (i < 5) {
        if (i % 2 == 0) {
            total = total + i;
        } else {
            total = total - 1;
        }
        i = i + 1;
    }

    return total;
}