int main(void) {
    int total = 0;
    for (int i = 0; i < 6; i = i + 1) {
        if (i == 2) i = i + 1;
        total = total + 1;
    }
    return total;
}
