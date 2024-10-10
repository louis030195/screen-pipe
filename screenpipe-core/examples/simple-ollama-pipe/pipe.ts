import { z } from "zod";
import { streamText } from "ai";
import { ollama } from "ollama-ai-provider";

const screenpipeQuery = z.object({
  q: z.string().optional(),
  content_type: z.enum(["ocr", "audio", "all"]).default("all"),
  limit: z.number().default(20),
  start_time: z.string().default(new Date(Date.now() - 3600000).toISOString()),
  end_time: z.string().default(new Date().toISOString()),
});

async function queryScreenpipe(params: z.infer<typeof screenpipeQuery>) {
  try {
    const queryParams = new URLSearchParams(
      Object.entries(params).filter(([_, v]) => v != null) as [string, string][]
    );
    console.log("Calling screenpipe", JSON.stringify(params));
    const response = await fetch(`http://localhost:3030/search?${queryParams}`);
    if (!response.ok) {
      throw new Error(`HTTP error! status: ${response.status}`);
    }
    return await response.json();
  } catch (error) {
    console.error("Error querying screenpipe:", error);
    return null;
  }
}

const simpleOllamaChat = async () => {
  console.log(
    "Starting simple Ollama chat. Make sure to run `ollama run nemotron-mini:4b-instruct-q4_K_M` before running this script."
  );
  const provider = ollama("nemotron-mini:4b-instruct-q4_K_M");

  while (true) {
    try {
      // query last 1 min of screenpipe
      const screenpipe = await queryScreenpipe({
        content_type: "all",
        limit: 10,
        start_time: new Date(Date.now() - 60000).toISOString(),
        end_time: new Date().toISOString(),
      });
      console.log(
        "got some screenpipe data of length:",
        screenpipe.data.length
      );

      const conversation: { role: string; content: string }[] = [
        {
          role: "user",
          content:
            "What did I do in the last minute? Here is the screenpipe data: " +
            JSON.stringify(screenpipe),
        },
      ];

      const { textStream } = await streamText({
        model: provider,
        messages: conversation,
        maxToolRoundtrips: 3,
      });

      process.stdout.write("ai: ");
      for await (const chunk of textStream) {
        process.stdout.write(chunk);
      }
      console.log(); // new line after the complete response
    } catch (error) {
      console.error("error in ollama chat:", error);
    }
  }
};

const main = async () => {
  await simpleOllamaChat();
};

main();
