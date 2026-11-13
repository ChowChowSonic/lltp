int main(void) {
    int total = 0;
    for (int i = 0; i < 4; i = i + 1) {
        if (i == 2) continue;
        total = total + 1;
    }
    return total;
}
