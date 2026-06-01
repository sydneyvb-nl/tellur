import './modelMetadata.test';
import './extension.test';
import { runRegisteredTests } from './harness';

export function run(): Promise<void> {
    return runRegisteredTests();
}
