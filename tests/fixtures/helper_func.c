int helper(int x) {
    int total = 0;
    while (x > 0) {
        total = total + x;
        x = x - 1;
    }
    return total;
}

int main(void) {
    return helper(5);
}