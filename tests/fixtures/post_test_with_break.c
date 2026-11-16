int main(void) {
    int i = 0;
    int total = 0;
    do {
        i = i + 1;
        if (i == 2) continue;
        if (i == 4) break;
        total = total + 1;
    } while (i < 5);
    return total;
}
