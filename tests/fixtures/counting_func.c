int count_up(int n) {
    int i = 0;
    while (i < n) {
        i = i + 1;
    }
    return i;
}

int count_down(int n) {
    int i = n;
    while (i > 0) {
        i = i - 1;
    }
    return i;
}

int main(void) {
    return count_up(3) + count_down(3);
}