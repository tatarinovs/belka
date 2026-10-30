/** An error for the user: what happened and what to do, with the raw text kept for details. */
export interface Explained {
  summary: string;
  /** The original message, set when `summary` is a rewording of it. */
  detail?: string;
}

const RULES: [RegExp, string][] = [
  [/http 401|http 403|unauthori[sz]ed|invalid_grant|token.*(expired|invalid|revoked)|sign-in/i, "Доступ к облаку закрыт или устарел. Откройте настройки аккаунта и войдите заново."],
  [/http 507|quota|insufficient storage|no space|not enough space|disk full/i, "В облаке или на диске не хватает места. Освободите место и повторите."],
  [/http 429|too many requests|rate limit/i, "Облако просит подождать: слишком много запросов. Повторите через минуту."],
  [/http 404|does not exist|not found/i, "Файл или папка не найдены. Возможно, их уже переместили или удалили. Обновите список."],
  [/already exists/i, "Такое имя уже занято. Выберите другое имя."],
  [/http 5\d\d/i, "Облако временно не отвечает. Повторите позже."],
  [/timed? ?out|connection (reset|refused|closed|aborted)|dns|network|error sending request|unreachable|broken pipe/i, "Нет связи с облаком. Проверьте интернет и повторите."],
  [/permission denied|access is denied|os error 5\b/i, "Нет доступа к файлу на компьютере. Закройте программу, которая его использует, и повторите."],
  [/wrong password|damaged/i, "Неверный пароль или файл повреждён."],
];

export function explainError(raw: string): Explained {
  const text = raw.trim();
  for (const [pattern, summary] of RULES) {
    if (pattern.test(text)) return { summary, detail: text };
  }
  return { summary: text };
}
