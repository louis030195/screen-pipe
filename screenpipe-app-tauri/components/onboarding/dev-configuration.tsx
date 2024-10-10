import React, { useEffect, useState } from "react";
import { ArrowUpRight } from "lucide-react";
import { open } from "@tauri-apps/plugin-shell";
import { platform } from "@tauri-apps/plugin-os";
import { CodeBlock } from "@/components/onboarding/single-codeblock";
import { DialogHeader, DialogTitle } from "@/components/ui/dialog";
import OnboardingNavigation from "@/components/onboarding/navigation";

interface OnboardingDevConfigProps {
  className?: string;
  handlePrevSlide: () => void;
  handleNextSlide: () => void;
}

interface devInstructionsItemsTypes {
  label: string;
  description: string;
  command: string;
}

type devInstructionItems = Record<string, devInstructionsItemsTypes[]>;

const OnboardingDevConfig: React.FC<OnboardingDevConfigProps> = ({
  className = "",
  handlePrevSlide,
  handleNextSlide,
}) => {
  const [instructions, setInstructions] = useState<devInstructionsItemsTypes[]>(
    []
  );

  const devInstructionsData: devInstructionItems = {
    windows: [
      {
        label: "to start using the screenpipe cli,",
        description: "to use the screenpipe cli, open your terminal with admin privileges and navigate to '%LOCALAPPDATA%\\screenpipe' or run this command to view all setup arguments",
        command: "cd %LOCALAPPDATA%\screenpipe && ./\screenpipe.exe -h   # shows list of arguments",
      },
      {
        label: "starting screenpipe with custom arguments,",
        description: "after reviewing the cli arguments, choose your setup options and start screenpipe with your preference. replace arguments as needed. for example:",
        command: "screenpipe --ignored-windows settings    # ignore the windows named settings",
      },
    ],
    macos: [
      {
        label: "to start using the screenpipe cli,",
        description: "to use the screenpipe cli, open your terminal and navigate to '/Applications/screenpipe.app/Contents/MacOS/' or run this command to view all setup arguments",
        command: "cd /Applications/screenpipe.app/Contents/MacOS/ && screenpipe -h  # shows help",
      },
      {
        label: "starting screenpipe with custom arguments",
        description: "after reviewing the cli arguments, choose your setup options and start screenpipe with your preference. replace arguments as needed. for example:",
        command: "screenpipe --list-monitors     # list monitors",
      },
    ],
    linux: [
      {
        label: "to start using the screenpipe cli,",
        description: "open your terminal and navigate to the installation directory (usually /usr/local/bin) or run this command, this will show all arguments to setup screenpipe as you prefer.",
        command: "cd /usr/local/bin/ && screenpipe -h   # shows list of arguments",
      },
      {
        label: "starting screenpipe with custom arguments",
        description: "after reviewing the cli arguments, choose your setup options and start screenpipe with your preference. replace arguments as needed. for example:",
        command: "screenpipe --ignored-windows kitty    # ignore the windows named kitty",
      },
    ],
  };

  useEffect(() => {
    const getOsType = () => {
      const os = platform();
      setInstructions(devInstructionsData[os] || []);
    };
    getOsType();
  }, []);

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
        <DialogTitle className="text-center !mt-[-4px] font-bold text-[30px] text-balance flex justify-center">
          screenpipe with dev config
        </DialogTitle>
        <h1 className="font-medium text-center !mt-[-1px] text-md">
          customize the screenpipe setting using dev configuration
        </h1>
      </DialogHeader>
      <div className="mt-2 w-full flex justify-around flex-col">
        <div className="mx-3">
          <p className="text-muted-foreground text-[14px]">
            <span className="font-medium prose text-[14px] mr-1">
              by using the cli,
            </span>
            you can manually configure and manage backend processes for advanced
            customization and debugging.
          </p>
        </div>
        {instructions.length > 0 && (
          <div className="mx-3 mt-1">
            <h1 className="font-medium text-md">
              run the screenpipe backend via cli:
            </h1>
            <ul className="mt-0">
              {instructions.map((instructions, index) => (
                <li key={index} className="list-disc mt-1">
                  <p className="text-muted-foreground text-sm ml-4">
                    <span className="font-medium text-nowrap text-[14px] mr-1 prose">
                      {instructions.label}
                    </span>
                    {instructions.description}
                  </p>
                  <CodeBlock
                    className="rounded-md mt-2"
                    language="bash"
                    value={instructions.command}
                  />
                </li>
              ))}
            </ul>
          </div>
        )}
        <a
          onClick={() => open("https://docs.screenpi.pe/")}
          href="#"
          className="mt-4 text-muted-foreground text-sm mr-auto ml-auto !text-center hover:underline"
        >
          learn more about screenpipe args &amp; api
          <ArrowUpRight className="inline w-4 h-4 ml-1 " />
        </a>
      </div>
      <OnboardingNavigation
        className="mt-6"
        handlePrevSlide={handlePrevSlide}
        handleNextSlide={handleNextSlide}
        prevBtnText="previous"
        nextBtnText="next"
      />
    </div>
  );
};

export default OnboardingDevConfig;
