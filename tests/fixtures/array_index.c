int main(void) {
    int arr[4];
    arr[0] = 1;
    arr[1] = 2;
    arr[2] = 3;
    arr[3] = 4;
    int sum = 0;
    for (int i = 0; i < 4; i++) {
        sum += arr[i];
    }
    arr[1] = sum;
    return arr[1] + arr[2];
}
