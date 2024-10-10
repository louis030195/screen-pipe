import React, { useState } from "react";
import { Info } from "lucide-react";
import { Wrench, UserRound } from "lucide-react";
import { useToast } from "@/components/ui/use-toast";
import { useSettings } from "@/lib/hooks/use-settings";
import { Card, CardContent } from "@/components/ui/card";
import { DialogHeader, DialogTitle } from "@/components/ui/dialog";
import OnboardingNavigation from "@/components/onboarding/navigation";

interface OnboardingDevOrNonDevProps {
  className?: string;
  handleNextSlide: () => void;
  handlePrevSlide: () => void;
  handleOptionClick: (option: string) => void;
  selectedPreference?: string | null;
}

const DEV_OPTIONS = [
  {
    key: "nonDevMode",
    icon: UserRound,
    title: "standard mode",
    description: "let screenpipe handle the backend automatically, ensuring a hands-free experience where complex operations are managed with precision and efficiency.",
  },
  {
    key: "devMode",
    icon: Wrench,
    title: "dev mode",
    description: "in development mode, gain full control over screenpipe, configure backend processes via cli, enabling you an advanced customization and independent testing.",
  },
];

const CardItem: React.FC<{
  isSelected: boolean;
  onClick: () => void;
  option: typeof DEV_OPTIONS[number];
}> = ({ option, isSelected, onClick }) => {
  const { icon: Icon, title, description } = option;

  return (
    <div className="relative group h-[270px]">
      <div className={`absolute h-full !mt-[-5px] inset-0 rounded-lg transition-all duration-300 ease-out group-hover:before:opacity-100 group-hover:before:scale-100 
        before:absolute before:inset-0 before:rounded-lg before:border-2 before:border-black dark:before:border-white before:opacity-0 before:scale-95 before:transition-all 
        before:duration-300 before:ease-out ${isSelected ? "before:!border-none" : "" }`}
      />
      <Card
        className={`p-4 h-[270px] !mt-[-5px] hover:bg-accent cursor-pointer bg-white dark:bg-gray-800 transition-all relative z-[1] duration-300 ease-out group-hover:scale-[0.98] 
        ${isSelected ? "bg-accent transition-transform relative border-2 border-black dark:border-white" : "" }`}
        onClick={onClick}
      >
        <CardContent className="flex flex-col w-[250px] justify-start">
          <Icon className="w-12 h-12 mx-auto" />
          <h2 className="font-semibold text-xl text-center mt-1">{title}</h2>
          <span className="prose-sm prose mt-0 text-balance">{description}</span>
        </CardContent>
      </Card>
    </div>
  );
};

const OnboardingDevOrNonDev: React.FC<OnboardingDevOrNonDevProps> = ({
  className = "",
  selectedPreference = "",
  handleOptionClick,
  handleNextSlide,
  handlePrevSlide,
}) => {

  const { toast } = useToast();
  const { settings, updateSettings } = useSettings();
  const [localSettings, setLocalSettings] = useState(settings);

  const handleOptionClickWithPreference = async (option: string) => {
    handleOptionClick(option);
    try {
      if (option === "devMode") {
        await updateSettings({ devMode: true })
        setLocalSettings({ ...localSettings, devMode: true });
          toast({
            title: "success",
            description: "dev mode enabled successfully",
            variant: "default"
          })
        } else if (option === "nonDevMode") {
          await updateSettings({ devMode: false });
          setLocalSettings({...localSettings, devMode: false});
          toast({
            title: "success",
            description: "screenpipe backend is in standard mode",
            variant: "default"
          })
        }
      } catch (error: any) {
        toast({
          title: "error",
          description: error,
          variant: "destructive"
        })
      }
  };

  return (
    <div className={`${className} w-full flex justify-center flex-col relative`}>
      <DialogHeader className="mt-1 px-2">
        <div className="w-full !mt-[-10px] inline-flex justify-center">
          <img src="/128x128.png" alt="screenpipe-logo" width="72" height="72" />
        </div>
        <DialogTitle className="text-center !mt-[-2px] font-bold text-[32px] text-balance flex justify-center">
          select your preference
        </DialogTitle>
        <p className="text-center text-lg">
          how do you prefer to use screenpipe?
        </p>
      </DialogHeader>
      <div className="flex w-full justify-around mt-4">
        {DEV_OPTIONS.map((option) => (
          <CardItem
            key={option.key}
            option={option}
            isSelected={selectedPreference === option.key}
            onClick={() => handleOptionClickWithPreference(option.key)}
          />
        ))}
      </div>
      <span className="absolute bottom-12 text-muted-foreground prose-sm text-center block w-full z-[-1]">
       <Info className="inline w-4 h-4 mb-[1px]" /> dev mode is for experts!
      </span>
      <OnboardingNavigation
        className="mt-9"
        handlePrevSlide={handlePrevSlide}
        handleNextSlide={handleNextSlide}
        prevBtnText="previous"
        nextBtnText="next"
      />
    </div>
  );
};

export default OnboardingDevOrNonDev;

