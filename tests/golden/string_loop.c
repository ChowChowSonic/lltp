int my_strlen(const char *s) {
    int n = 0;
    while (*s != '\0') {
        s++;
        n++;
    }
    return n;
}
int main(void) { return my_strlen("hello"); }