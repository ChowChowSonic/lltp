struct P { int x; char c; };
int main(void) {
    struct P p;
    p.x = 3;
    p.c = 'z';
    return p.x + p.c;
}
