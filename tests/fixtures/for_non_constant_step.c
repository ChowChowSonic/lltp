int main(void) {
    int total = 0;
    int k = 1;
    for (int i = 0; i < 4; i = i + k) {
        total = total + 1;
    }
    return total;
}
