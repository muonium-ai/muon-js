function fib(n){ if(n<=0) return 0; else if(n===1) return 1; else return fib(n-1)+fib(n-2);} 
var fib_func = function fib1(n){ if(n<=0) return 0; else if(n==1) return 1; else return fib1(n-1)+fib1(n-2); };
console.log(fib(6), fib_func(6));
