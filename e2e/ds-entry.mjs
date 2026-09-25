import { Datastar as DS } from './node_modules/@starfederation/datastar/dist/engine/index.js';
import { SSE } from './node_modules/@starfederation/datastar/dist/plugins/official/backend/actions/sse.js';
import { Indicator } from './node_modules/@starfederation/datastar/dist/plugins/official/backend/attributes/indicator.js';
import { ExecuteScript } from './node_modules/@starfederation/datastar/dist/plugins/official/backend/watchers/executeScript.js';
import { MergeFragments } from './node_modules/@starfederation/datastar/dist/plugins/official/backend/watchers/mergeFragments.js';
import { MergeSignals } from './node_modules/@starfederation/datastar/dist/plugins/official/backend/watchers/mergeSignals.js';
import { RemoveFragments } from './node_modules/@starfederation/datastar/dist/plugins/official/backend/watchers/removeFragments.js';
import { RemoveSignals } from './node_modules/@starfederation/datastar/dist/plugins/official/backend/watchers/removeSignals.js';
import { Clipboard } from './node_modules/@starfederation/datastar/dist/plugins/official/browser/actions/clipboard.js';
import { Intersects } from './node_modules/@starfederation/datastar/dist/plugins/official/browser/attributes/intersects.js';
import { Persist } from './node_modules/@starfederation/datastar/dist/plugins/official/browser/attributes/persist.js';
import { ReplaceUrl } from './node_modules/@starfederation/datastar/dist/plugins/official/browser/attributes/replaceUrl.js';
import { ScrollIntoView } from './node_modules/@starfederation/datastar/dist/plugins/official/browser/attributes/scrollIntoView.js';
import { Show } from './node_modules/@starfederation/datastar/dist/plugins/official/browser/attributes/show.js';
import { ViewTransition } from './node_modules/@starfederation/datastar/dist/plugins/official/browser/attributes/viewTransition.js';
import { Attributes } from './node_modules/@starfederation/datastar/dist/plugins/official/dom/attributes/attributes.js';
import { Bind } from './node_modules/@starfederation/datastar/dist/plugins/official/dom/attributes/bind.js';
import { Class } from './node_modules/@starfederation/datastar/dist/plugins/official/dom/attributes/class.js';
import { On } from './node_modules/@starfederation/datastar/dist/plugins/official/dom/attributes/on.js';
import { Ref } from './node_modules/@starfederation/datastar/dist/plugins/official/dom/attributes/ref.js';
import { Text } from './node_modules/@starfederation/datastar/dist/plugins/official/dom/attributes/text.js';
import { Fit } from './node_modules/@starfederation/datastar/dist/plugins/official/logic/actions/fit.js';
import { SetAll } from './node_modules/@starfederation/datastar/dist/plugins/official/logic/actions/setAll.js';
import { ToggleAll } from './node_modules/@starfederation/datastar/dist/plugins/official/logic/actions/toggleAll.js';

DS.load(
  Bind,
  Indicator,
  Ref,
  Attributes,
  Class,
  On,
  Show,
  Text,
  SSE,
  MergeFragments,
  MergeSignals,
  RemoveFragments,
  RemoveSignals,
  ExecuteScript,
  Clipboard,
  Intersects,
  Persist,
  ReplaceUrl,
  ScrollIntoView,
  ViewTransition,
  Fit,
  SetAll,
  ToggleAll,
);

export const Datastar = DS;
