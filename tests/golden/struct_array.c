struct P { int x; char c; };
int main(void) {
    struct P p = { 3, 'z' };
    int arr[4] = {1, 2, 3, 4};
    return p.x + arr[2] + p.c;
}