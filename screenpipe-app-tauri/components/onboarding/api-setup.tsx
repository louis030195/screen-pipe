import React from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { useSettings } from "@/lib/hooks/use-settings";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Eye, EyeOff, HelpCircle, ArrowUpRight } from "lucide-react";
import { DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { open } from "@tauri-apps/plugin-shell";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import OnboardingNavigation from "@/components/onboarding/navigation";

interface OnboardingAPISetupProps {
  className?: string;
  handleNextSlide: () => void;
  handlePrevSlide: () => void;
}

const OnboardingAPISetup: React.FC<OnboardingAPISetupProps> = ({ 
  className,
  handleNextSlide,
  handlePrevSlide,
}) => {

  const { settings, updateSettings } = useSettings();
  const [localSettings, setLocalSettings] = React.useState(settings);
  const [showApiKey, setShowApiKey] = React.useState(false);

  const handleApiUrlChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const newValue = e.target.value;
    setLocalSettings({ ...localSettings, aiUrl: newValue });
    updateSettings({ aiUrl: newValue });
  };

  const handleApiKeyChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const newValue = e.target.value;
    setLocalSettings({ ...localSettings, openaiApiKey: newValue });
    updateSettings({ openaiApiKey: newValue });
  };

  const handleModelChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const newValue = e.target.value;
    setLocalSettings({ ...localSettings, aiModel: newValue });
    updateSettings({ aiModel: newValue });
  };

  React.useEffect(() => {
    setLocalSettings(settings);
  }, [settings]);

  return (
    <div className={`flex h-[80%] flex-col ${className}`}>
      <DialogHeader
        className={`flex justify-center items-center`}
      >
        <div className="w-full !mt-[-10px] inline-flex justify-center">
          <img
            src="/128x128.png"
            alt="screenpipe-logo"
            width="72"
            height="72"
          />
        </div>
        <DialogTitle className="font-bold text-[28px] text-balance">
          Add API key to use AI-Enhanced Summarization
        </DialogTitle>
      </DialogHeader>
      <Card className="mt-2">
        <CardHeader>
          <CardTitle className="text-center">Setup API Key</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col items-center space-y-4">
          <div className="w-full">
            <div className="flex items-center gap-2 mb-2">
              <Label htmlFor="aiUrl" className="min-w-[80px] text-right">
                AI URL
              </Label>
              <div className="flex-grow flex items-center">
                <Input
                  id="aiUrl"
                  value={localSettings.aiUrl}
                  onChange={handleApiUrlChange}
                  className="flex-grow"
                  placeholder="Enter AI URL"
                  type="url"
                />
                <TooltipProvider>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <HelpCircle className="ml-2 h-4 w-4 cursor-default" />
                    </TooltipTrigger>
                    <TooltipContent side="left">
                      <p>
                        The url of your ai provider&apos;s api endpoint. for
                        openai:{" "}
                        <pre className="bg-gray-100 p-1 rounded-md">
                          https://api.openai.com/v1
                        </pre>
                        <br />
                        For local providers like ollama usually it&apos;s
                        <pre className="bg-gray-100 p-1 rounded-md">
                          http://localhost:11434/v1
                        </pre>
                        <br />
                        Note: on windows, you may need to run ollama with:
                        <pre className="bg-gray-100 p-1 rounded-md">
                          OLLAMA_ORIGINS=* ollama run llama3.2
                        </pre>
                      </p>
                    </TooltipContent>
                  </Tooltip>
                </TooltipProvider>
              </div>
            </div>
          </div>
          <div className="w-full">
            <div className="flex items-center gap-2 mb-2">
              <Label htmlFor="aiApiKey" className="min-w-[80px] text-right">
                API key
              </Label>
              <div className="flex-grow relative">
                <Input
                  id="aiApiKey"
                  type={showApiKey ? "text" : "password"}
                  value={localSettings.openaiApiKey}
                  onChange={handleApiKeyChange}
                  className="pr-10"
                  placeholder="Enter Your AI API key"
                  autoCorrect="off"
                  autoCapitalize="off"
                  autoComplete="off"
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="absolute right-0 top-0 h-full"
                  onClick={() => setShowApiKey(!showApiKey)}
                >
                  {showApiKey ? (
                    <EyeOff className="h-4 w-4" />
                  ) : (
                    <Eye className="h-4 w-4" />
                  )}
                </Button>
              </div>
            </div>
          </div>
          <div className="w-full">
            <div className="flex items-center gap-4 mb-4">
              <Label htmlFor="aiModel" className="min-w-[80px] text-right">
                AI model
              </Label>
              <Input
                id="aiModel"
                value={localSettings.aiModel}
                onChange={handleModelChange}
                className="flex-grow"
                placeholder="Enter AI Model (e.g., gpt-4)"
                autoCorrect="off"
                autoCapitalize="off"
                autoComplete="off"
              />
            </div>
          </div>
        </CardContent>
      </Card>
      <a
        onClick={() =>
          open(
            "https://github.com/ollama/ollama?tab=readme-ov-file#ollama",
          )
        }
        href="#"
        className="mt-4 text-muted-foreground text-sm mr-auto ml-auto !text-center hover:underline"
      >
        Don&apos;t have API key ? Set up Ollama Locally
        <ArrowUpRight className="inline w-4 h-4 ml-1 " />
      </a>
      <OnboardingNavigation 
        className="mt-8"
        handlePrevSlide={handlePrevSlide}
        handleNextSlide={handleNextSlide}
        prevBtnText="Previous"
        nextBtnText="Setup"
      />
    </div>
  );
};

export default OnboardingAPISetup;

