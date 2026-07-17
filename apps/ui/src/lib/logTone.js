const BAD_PATTERNS = [
  /silent\s*error/i,
  /device\s*lost/i,
  /\btdr\b/i,
  /\bcrash(?:ed)?\b/i,
  /\bunstable\b/i,
  /\bnot\s+stable\b/i,
  /\bblacklist(?:ed)?\b/i,
  /\bdisqualif(?:ied|ication)\b/i,
  /\bfail(?:ed|ure)?\b/i,
  /\berror\b/i,
  /\babort(?:ed)?\b/i,
  /\brefus(?:ed|al)\b/i,
  /\binconclusive\b/i,
  /\bwarning\b/i,
  /\bwarn\b/i,
  /\bunsafe\b/i,
  /\brejected?\b/i,
  /\berro\b/i,
  /\bfalh(?:a|ou|ado|ada|aram)\b/i,
  /\binst[aá]vel\b/i,
  /\bdesqualificad[oa]\b/i,
  /\brejeitad[oa]\b/i,
  /\brecusad[oa]\b/i,
  /\binconclusiv[oa]\b/i,
  /\bavisos?\b/i,
  /\babortad[oa]\b/i,
  /\bn[aã]o\s+(?:confirmad[oa]|validado|qualificado|aprovado)\b/i,
];

const GOOD_PATTERNS = [
  /\bvalidated?\b/i,
  /\bqualified?\b/i,
  /\bapproved?\b/i,
  /\bpassed?\b/i,
  /\bstable\b/i,
  /\bsuccess(?:ful(?:ly)?)?\b/i,
  /\bcomplete(?:d)?\b/i,
  /\bforged\b/i,
  /\bsaved\b/i,
  /\brecovered?\b/i,
  /\breset\s+(?:confirmed|clean|ok)\b/i,
  /\b0\s+(?:errors?|mismatches?)\b/i,
  /(?:^|\s)ok(?:\s|[.!,:;]|$)/i,
  /\bvalidad[oa]\b/i,
  /\bqualificad[oa]\b/i,
  /\baprova(?:do|da)\b/i,
  /\best[aá]vel\b/i,
  /\bsucesso\b/i,
  /\bconclu[ií]d[oa]\b/i,
  /\bsalv[oa]\b/i,
  /\brecuperad[oa]\b/i,
  /\breset\s+(?:confirmado|limpo|ok)\b/i,
  /\b0\s+(?:erros?|diverg[eê]ncias?)\b/i,
];

/**
 * Classifies a Forge log line for presentation only. Safety-sensitive/bad
 * evidence wins when a line also contains a positive cleanup result.
 */
export function classifyForgeLogLine(line) {
  const value = String(line ?? "");
  if (BAD_PATTERNS.some((pattern) => pattern.test(value))) return "bad";
  if (GOOD_PATTERNS.some((pattern) => pattern.test(value))) return "good";
  return "process";
}
