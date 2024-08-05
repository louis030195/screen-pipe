import { z } from "zod";
import { openai } from "@ai-sdk/openai";
import { generateObject, generateText, streamText } from "ai";
import { queryScreenpipe, ContentType } from "./screenpipe";
import * as fs from "fs/promises";
console.log("Starting daily tracker application");

const CHUNK_SIZE = 5 * 60; // 5 minutes in seconds
console.log(`CHUNK_SIZE set to ${CHUNK_SIZE} seconds`);

const ActivitySchema = z.object({
  category: z.string(),
  description: z.string(),
  duration: z.number(),
});

type Activity = z.infer<typeof ActivitySchema>;

interface Usage {
  [key: string]: number;
}

interface AnalysisResult {
  appUsage: Usage;
  activities: Activity[];
}

const analyzeTimeRangeSchema = z.object({
  startTime: z.string(),
  endTime: z.string(),
});

const extractActivitySchema = z.object({
  entry: z.any(),
});

const generateSummarySchema = z.object({
  analysisResult: z.any(),
});

const streamResponseSchema = z.object({
  response: z.string(),
});

async function timeTracker(userInput: string) {
  console.log(`Starting timeTracker with user input: "${userInput}"`);
  try {
    const text = await generateText({
      model: openai("gpt-4o"),
      tools: {
        analyze_time_range: {
          description:
            "Analyze app usage and activities within a given time range",
          parameters: analyzeTimeRangeSchema,
          execute: async ({ startTime, endTime }) => {
            console.log(`Analyzing time range: ${startTime} to ${endTime}`);
            let appUsage: Usage = {};
            let activities: Activity[] = [];

            // Use the current local date
            const now = new Date();
            const startOfDay = new Date(now.setHours(0, 0, 0, 0));
            const endOfDay = new Date(now.setHours(23, 59, 59, 999));

            let currentTime = startOfDay;
            const endTimeDate = endOfDay;

            while (currentTime < endTimeDate) {
              const chunkEnd = new Date(
                Math.min(
                  currentTime.getTime() + CHUNK_SIZE * 1000,
                  endTimeDate.getTime()
                )
              );
              console.log(
                `Querying Screenpipe for chunk: ${currentTime.toISOString()} to ${chunkEnd.toISOString()}`
              );
              const chunkData = await queryScreenpipe({
                start_time: currentTime.toISOString(),
                end_time: chunkEnd.toISOString(),
                offset: 0,
                limit: 1000,
                content_type: "all" as ContentType,
              });

              console.log(
                `Received ${chunkData.data.length} entries from Screenpipe`
              );
              for (const entry of chunkData.data) {
                if (entry.type === "OCR") {
                  const app = entry.content.app_name.toLowerCase();
                  appUsage[app] = (appUsage[app] || 0) + 1;
                }
              }
              currentTime = chunkEnd;
            }

            appUsage = Object.fromEntries(
              Object.entries(appUsage).map(([app, seconds]) => [
                app,
                seconds / 3600,
              ])
            );

            console.log("App usage analysis completed");
            console.log("App usage summary:", appUsage);

            const analysisResult: AnalysisResult = { appUsage, activities };
            await fs.writeFile(
              "analysis_result.json",
              JSON.stringify(analysisResult, null, 2)
            );
            console.log("Analysis result saved to analysis_result.json");
            return analysisResult;
          },
        },
        extract_activity: {
          description:
            "Extract activity information from a single entry and update the analysis result",
          parameters: extractActivitySchema,
          execute: async ({ entry }) => {
            console.log("Extracting activity from entry:", entry);
            const prompt = `
            Based on the following screen capture or audio transcription data, categorize the activity and provide a brief description.
            Data: ${JSON.stringify(entry)}
            
            Respond with a JSON object in this format:
            {
              "category": "Work/Personal/Entertainment/etc.",
              "description": "Brief description of the activity",
              "duration": 1
            }
            
            The duration should always be 1 (representing 1 second).
          `;

            console.log("Sending prompt to GPT-4 for activity extraction");
            const response = await generateObject({
              model: openai("gpt-4o"),
              messages: [{ role: "user", content: prompt }],
              schema: ActivitySchema,
            });

            try {
              const activity = response.object;
              console.log(
                "Extracted activity:",
                JSON.stringify(activity, null, 2)
              );
              const analysisResult = JSON.parse(
                await fs.readFile("analysis_result.json", "utf-8")
              );
              analysisResult.activities.push(activity);
              await fs.writeFile(
                "analysis_result.json",
                JSON.stringify(analysisResult, null, 2)
              );
              console.log("Updated analysis result with new activity");
              return activity;
            } catch (error) {
              console.error("Failed to parse activity:", error);
              return null;
            }
          },
        },
        generate_summary: {
          description:
            "Generate a summary of time usage based on the analysis result",
          parameters: generateSummarySchema,
          execute: async ({ analysisResult }) => {
            console.log("Generating summary from analysis result");
            const { appUsage, activities } = analysisResult;
            const topApps = Object.entries(appUsage)
              .sort((a, b) => b[1] - a[1])
              .slice(0, 5);

            const categorySummary = activities.reduce((acc, activity) => {
              acc[activity.category] =
                (acc[activity.category] || 0) + activity.duration;
              return acc;
            }, {} as Usage);

            const topCategories = Object.entries(categorySummary)
              .sort((a, b) => b[1] - a[1])
              .slice(0, 5);

            console.log("Top 5 apps:", topApps);
            console.log("Top 5 categories:", topCategories);

            const prompt = `
            Based on the following data:
            
            Top 5 apps used (in hours, need to multiply by 10):
            ${JSON.stringify(topApps)}
            
            Top 5 activity categories (in seconds):
            ${JSON.stringify(topCategories)}
            
            Provide a summary of how the user spent their time this week.
            Focus on answering the question: "What's the thing I spend the most time on this week?"
            Include insights about both app usage and activity categories.
            Be concise but informative.
          `;

            console.log("Sending prompt to GPT-4 for summary generation");
            return generateText({
              model: openai("gpt-4o"),
              messages: [{ role: "user", content: prompt }],
            });
          },
        },
        stream_response: {
          description: "Stream the final response to the user",
          parameters: streamResponseSchema,
          execute: async ({ response }) => {
            console.log("Streaming response to user");
            const { textStream } = await streamText({
              model: openai("gpt-4o"),
              messages: [
                {
                  role: "user",
                  content: response,
                },
              ],
            });

            let buffer = "";
            const flushBuffer = () => {
              if (buffer) {
                process.stdout.write(buffer);
                buffer = "";
              }
            };

            for await (const chunk of textStream) {
              buffer += chunk;
              if (buffer.includes(" ") && buffer.length > 20) {
                flushBuffer();
              }
            }
            flushBuffer(); // Flush any remaining content
            console.log("\nFinished streaming response");
            throw new Error("STREAM_COMPLETE");
          },
        },
      },
      toolChoice: "required",
      messages: [
        {
          role: "system",
          content: `You are a helpful assistant that analyzes time usage data.
        The user wants to know what they spent the most time on this week.
        Use the provided tools to analyze the data and generate a summary.
        Maintain and update the analysis result in the JSON file throughout the process.
        Always use the stream_response tool to output the final result to the user. Stop after streaming your final response.
        
        Rules:
        - Do not use Markdown as the answer will be streamed in a terminal`,
        },
        {
          role: "user",
          content: userInput,
        },
      ],
      maxToolRoundtrips: 5,
    });

    console.log("Final response from GPT-4:", text);
  } catch (error) {
    if (error instanceof Error && error.message === "STREAM_COMPLETE") {
      console.log("Streaming completed, exiting timeTracker");
      return;
    }
    throw error; // Re-throw other errors
  }
}

const main = async () => {
  // Example usage
  const startTime = new Date();
  console.log(`Starting example usage at ${startTime.toISOString()}`);
  await timeTracker("What did I spend the most time on this week?");
  console.log("Example usage completed");
  const endTime = new Date();
  console.log(`Example usage completed at ${endTime.toISOString()}`);
  console.log(
    `Example usage took ${endTime.getTime() - startTime.getTime()}ms`
  );
};

main();
