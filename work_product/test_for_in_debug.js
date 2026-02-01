function assert(actual, expected, message) {
    if (arguments.length == 1)
        expected = true;
    if (actual === expected)
        return;
    if (actual !== null && expected !== null
    &&  typeof actual == 'object' && typeof expected == 'object'
    &&  actual.toString() === expected.toString())
        return;
    throw Error("assertion failed: got |" + actual + "|" +
                ", expected |" + expected + "|" +
                (message ? " (" + message + ")" : ""));
}

function test_for_in()
{
    var i, tab;

    tab = [];
    for(i in {x:1, y: 2}) {
        tab.push(i);
    }
    console.log("tab: " + tab);
    console.log("tab.toString(): " + tab.toString());
    assert(tab.toString(), "x,y", "for_in");
}

test_for_in();
console.log("passed");
