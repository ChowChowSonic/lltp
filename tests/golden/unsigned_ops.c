int main(void) {
    unsigned int a = 10, b = 3;
    unsigned int q = a / b;
    if (a > b)
        return (int)(q + a % b);
    return 0;
}