function cb(param){ return "test" + param; }
var r1 = new Rectangle(100,200);
console.log("r1", r1.x, r1.y);
var r2 = new FilledRectangle(100,200,0x123456);
console.log("r2", r2.x, r2.y, r2.color);
var func = Rectangle.getClosure("abcd");
console.log("closure", func());
console.log("call", Rectangle.call(cb,"abc"));
