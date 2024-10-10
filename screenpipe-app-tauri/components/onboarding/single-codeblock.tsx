"use client";

import { FC, memo } from "react";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { coldarkDark } from "react-syntax-highlighter/dist/cjs/styles/prism";
import { useCopyToClipboard } from "@/lib/hooks/use-copy-to-clipboard";
import { IconCheck, IconCopy} from "@/components/ui/icons";
import { Button } from "@/components/ui/button";

interface Props {
  className?: string;
  language: string;
  value: string;
}

interface languageMap {
  [key: string]: string | undefined;
}

export const programmingLanguages: languageMap = {
  javascript: ".js",
  python: ".py",
  java: ".java",
  c: ".c",
  cpp: ".cpp",
  "c++": ".cpp",
  "c#": ".cs",
  ruby: ".rb",
  php: ".php",
  swift: ".swift",
  "objective-c": ".m",
  kotlin: ".kt",
  typescript: ".ts",
  go: ".go",
  perl: ".pl",
  rust: ".rs",
  scala: ".scala",
  haskell: ".hs",
  lua: ".lua",
  shell: ".sh",
  sql: ".sql",
  html: ".html",
  css: ".css",
};

export const generateRandomString = (length: number, lowercase = false) => {
  const chars = "ABCDEFGHJKLMNPQRSTUVWXY3456789";
  let result = "";
  for (let i = 0; i < length; i++) {
    result += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return lowercase ? result.toLowerCase() : result;
};

const CodeBlock: FC<Props> = memo(({ className, language, value }) => {
  const { isCopied, copyToClipboard } = useCopyToClipboard({ timeout: 2000 });

  const onCopy = () => {
    if (isCopied) return;
    copyToClipboard(value);
  };

  return (
    <div className={` relative w-full font-sans codeblock backdrop-blur-sm bg-zinc-950 ${className}`}>
      <SyntaxHighlighter
        language={language}
        style={coldarkDark}
        PreTag="div"
        customStyle={{
          margin: 0,
          width: "100%",
          background: "transparent",
          padding: "10px 8px",
        }}
        codeTagProps={{
          style: {
            fontSize: "0.85rem",
            fontFamily: "var(--font-mono)",
          },
        }}
      >
        {value}
      </SyntaxHighlighter>
      <div className="absolute rounded-md z-[100] items-center right-0 top-[2px] text-zinc-100 bg-zinc-950">
        <Button
          title="Copy"
          variant="ghost"
          size="icon"
          className="text-xs hover:text-zinc-200/80 hover:bg-zinc-950"
          onClick={onCopy}
        >
          {isCopied ? <IconCheck /> : <IconCopy />}
          <span className="sr-only">Copy code</span>
        </Button>
      </div>
    </div>
  );
});
CodeBlock.displayName = "CodeBlock";

export { CodeBlock };
