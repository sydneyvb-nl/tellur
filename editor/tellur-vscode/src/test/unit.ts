import './suite/modelMetadata.test';
import './suite/editorIdentity.test';
import { runRegisteredTests } from './suite/harness';

runRegisteredTests().catch(error => {
    console.error(error);
    process.exit(1);
});
