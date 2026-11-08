package main

// the same suite as suite.c and suite.py

import "fmt"

func fib(n int) int {
	if n < 2 {
		return n
	}
	return fib(n-1) + fib(n-2)
}

func gcd(a, b int) int {
	for b != 0 {
		t := a % b
		a = b
		b = t
	}
	return a
}

func isPrime(n int) bool {
	if n < 2 {
		return false
	}
	for d := 2; d*d <= n; d++ {
		if n%d == 0 {
			return false
		}
	}
	return true
}

func collatz(n int) int {
	steps := 0
	for n != 1 {
		if n%2 == 0 {
			n = n / 2
		} else {
			n = 3*n + 1
		}
		steps++
	}
	return steps
}

func sum6(a, b, c, d, e, f int) int {
	return a + 2*b + 3*c + 4*d + 5*e + 6*f
}

func main() {
	fmt.Println("fib", fib(25))
	fmt.Println("gcd", gcd(1071, 462))
	count := 0
	for i := 0; i < 10000; i++ {
		if isPrime(i) {
			count++
		}
	}
	fmt.Println("primes", count)
	fmt.Println("collatz", collatz(27))
	fmt.Println("sum6", sum6(1, 2, 3, 4, 5, 6))
	fmt.Printf("div %d %d %d %d\n", -7/2, -7%2, 7/-2, 7%-2)
	fmt.Println("lt", 3 < 4, 4 < 3)
	fmt.Println("min", -9223372036854775807-1)
	pairs := 0
	for i := 0; i < 10; i++ {
		if i == 7 {
			break
		}
		for j := 0; j < 10; j++ {
			if j%3 == 0 {
				continue
			}
			pairs += i * j
		}
	}
	fmt.Println("pairs", pairs)
	fmt.Println("done")
}
