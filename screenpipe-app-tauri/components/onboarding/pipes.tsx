import React from "react";
import { CodeBlock } from "@/components/onboarding/single-codeblock"
import { ArrowUpRight } from "lucide-react";
import { DialogHeader, DialogTitle } from "@/components/ui/dialog";
import OnboardingNavigation from "@/components/onboarding/navigation";
import { open } from "@tauri-apps/plugin-shell";

interface OnboardingPipesProps {
  className?: string;
  handlePrevSlide: () => void;
  handleNextSlide: () => void;
}

const OnboardingPipes: React.FC<OnboardingPipesProps> = ({
  className = "",
  handlePrevSlide,
  handleNextSlide,
}) => {
  return (
    <div className={`${className} w-full flex justify-center flex-col`}>
      <DialogHeader className="px-2">
        <div className="w-full inline-flex !mt-[-10px] justify-center">
          <img
            src="/128x128.png"
            alt="screenpipe-logo"
            width="72"
            height="72"
          />
        </div>
        <DialogTitle className="text-center !mt-[-3px] font-bold text-[30px] text-balance flex justify-center">
          screenpipe with custom pipes (plugins)
        </DialogTitle>
        <h1 className="font-medium text-center !mt-[-1px] text-md prose">
          unlock screenpipe’s power with customizable pipes
        </h1>
      </DialogHeader>
      <div className="mt-2 w-full flex justify-around flex-col">
        <div className="mx-3">
          <p className="text-muted-foreground text-[14px]">
            <span className="font-medium text-nowrap text-[14px] prose mr-1">
              screenpipe is built to be fully extensible,
            </span> 
            allowing you to enhance its capabilities with custom
            pipes, versatile plugins that streamline workflow automation 
            for analyzing, managing your captured data.
          </p>
        </div>
        <div className="mx-3 mt-1">
          <h1 className="font-semibold text-md">
            get started with pipes:
          </h1>
          <ul className="mt-1">
            <li className="list-disc">
              <p className="text-muted-foreground text-sm ml-4">
                <span className="font-medium text-nowrap text-[14px] mr-1 prose">
                  download a pipe:
                </span> 
                  you can download a pipe from our github repository directly via cli
              </p>
              <CodeBlock 
                className="rounded-md mt-2"
                language="bash"
                value="screenpipe pipe download https://github.com/mediar-ai/screenpipe/tree/main/examples/typescript/pipe-stream-ocr-text"
              />
            </li>
            <li className="mt-2 list-disc">
              <p className="text-muted-foreground text-sm ml-4">
                <span className=" font-medium text-nowrap text-[14px] mr-1 prose">
                  enable the pipe:  
                </span> 
                  once downloaded, enable the pipe to activate it &amp; restart the application,
                  this pipe will stream text to local markdown files in your current directory.
              </p>
              <CodeBlock 
                className="rounded-md mt-2"
                language="bash"
                value="screenpipe pipe enable pipe-stream-ocr-text"
              />
            </li>
          </ul>
        </div>
        <a
          onClick={() =>
            open(
              "https://docs.screenpi.pe/docs/plugins#quick-tour---developing-pipes-in-screenpipe",
            )
          }
          href="#"
          className="mt-4 text-muted-foreground text-sm mr-auto ml-auto !text-center hover:underline"
        >
          checkout our docs for creating your own pipe!
          <ArrowUpRight className="inline w-4 h-4 ml-1 " />
        </a>
      </div>
      <OnboardingNavigation
        className="mt-8"
        handlePrevSlide={handlePrevSlide}
        handleNextSlide={handleNextSlide}
        prevBtnText="previous"
        nextBtnText="next"
      />
    </div>
  );
};

export default OnboardingPipes;

