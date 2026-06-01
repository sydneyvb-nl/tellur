interface RegisteredTest {
    suite: string;
    name: string;
    run: () => unknown | Promise<unknown>;
}

const tests: RegisteredTest[] = [];
let currentSuite = '';

export function suite(name: string, define: () => void): void {
    const previous = currentSuite;
    currentSuite = name;
    define();
    currentSuite = previous;
}

export function test(name: string, run: () => unknown | Promise<unknown>): void {
    tests.push({ suite: currentSuite, name, run });
}

export async function runRegisteredTests(): Promise<void> {
    let failures = 0;
    for (const registered of tests) {
        const label = `${registered.suite} - ${registered.name}`;
        try {
            await registered.run();
            console.log(`PASS ${label}`);
        } catch (error) {
            failures += 1;
            console.error(`FAIL ${label}`);
            console.error(error);
        }
    }

    if (failures > 0) {
        throw new Error(`${failures} test(s) failed.`);
    }
}
