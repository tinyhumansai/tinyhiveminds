Starting with two strings S_0 = 0 and S_1 = 01, define S_n as the
concatenation S_(n-1)S_(n-2) for n >= 2.

For example, S_2 = 010, S_3 = 01001, and S_4 = 01001010.

A string is called a Fibonacci subword if it is a contiguous substring of
some S_n. For every positive integer k there are exactly k+1 different
Fibonacci subwords of length k. Interpret each as a decimal number, ignoring
leading zeroes, and let Psi(k) be the sum of their squares.

For k = 3 the four subwords are 001, 010, 100, and 101, so
Psi(3) = 20302. You are also given
Psi(10) = 10699667 (mod 101001001).

Find Psi(10^18) mod 101001001.
