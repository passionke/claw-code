/// <reference types="vite/client" />

/** Chrome 138+ on-device translation (optional). Author: kejiqing */
interface TranslatorInstance {
  translate(input: string): Promise<string>;
}

interface TranslatorConstructor {
  create(options: { sourceLanguage: string; targetLanguage: string }): Promise<TranslatorInstance>;
}

interface Window {
  Translator?: TranslatorConstructor;
}
