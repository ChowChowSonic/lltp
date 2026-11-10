int main(void) {
    int total = 0;
    int i = 0;
    while (i < 3) {
        int j = 0;
        do {
            total = total + 1;
            j = j + 1;
        } while (j < 2);
        i = i + 1;
    }
    return total;
}
